use crate::util;
use anyhow::{anyhow, bail, Result};
use normpath::PathExt;
use regex::Regex;
use semver::Version;
use std::{
    cell::RefCell,
    fs::{self, File},
    io::{Cursor, Read, Seek, Write},
    path::{Path, PathBuf},
    rc::Rc,
};
use xml::reader::{EventReader, XmlEvent};
use zip::ZipArchive;

#[cfg(target_os = "macos")]
use std::os::unix::fs::PermissionsExt;

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

#[derive(Clone)]
pub struct BundleInfo<'a> {
    zip: Rc<RefCell<ZipArchive<Box<dyn ReadSeek + 'a>>>>,
    zip_from_file: bool,
    zip_range: Option<&'a [u8]>,
    file_path: Option<PathBuf>,
}

pub fn load_bundle_from_file<'a, P: AsRef<Path>>(file_name: P) -> Result<BundleInfo<'a>> {
    let file_name = file_name.as_ref();
    debug!("Loading bundle from file '{}'...", file_name.to_string_lossy());
    let file = util::retry_io(|| File::open(&file_name))?;
    let cursor: Box<dyn ReadSeek> = Box::new(file);
    let zip = ZipArchive::new(cursor)?;
    return Ok(BundleInfo { zip: Rc::new(RefCell::new(zip)), zip_from_file: true, file_path: Some(file_name.to_owned()), zip_range: None });
}

impl BundleInfo<'_> {
    pub fn calculate_size(&self) -> (u64, u64) {
        let mut total_uncompressed_size = 0u64;
        let mut total_compressed_size = 0u64;
        let mut archive = self.zip.borrow_mut();

        for i in 0..archive.len() {
            let file = archive.by_index(i);
            if file.is_ok() {
                let file = file.unwrap();
                total_uncompressed_size += file.size();
                total_compressed_size += file.compressed_size();
            }
        }

        (total_compressed_size, total_uncompressed_size)
    }

    pub fn get_splash_bytes(&self) -> Option<Vec<u8>> {
        let splash_idx = self.find_zip_file(|name| name.contains("splashimage"));
        if splash_idx.is_none() {
            warn!("Could not find splash image in bundle.");
            return None;
        }

        let mut archive = self.zip.borrow_mut();
        let sf = archive.by_index(splash_idx.unwrap());
        if sf.is_err() {
            warn!("Could not find splash image in bundle.");
            return None;
        }

        let res: Result<Vec<u8>, _> = sf.unwrap().bytes().collect();
        if res.is_err() {
            warn!("Could not find splash image in bundle.");
            return None;
        }

        let bytes = res.unwrap();
        if bytes.is_empty() {
            warn!("Could not find splash image in bundle.");
            return None;
        }

        Some(bytes)
    }

    pub fn find_zip_file<F>(&self, predicate: F) -> Option<usize>
    where
        F: Fn(&str) -> bool,
    {
        let mut archive = self.zip.borrow_mut();
        for i in 0..archive.len() {
            if let Ok(file) = archive.by_index(i) {
                let name = file.name();
                if predicate(name) {
                    return Some(i);
                }
            }
        }
        None
    }

    pub fn extract_zip_idx_to_path<T: AsRef<Path>>(&self, index: usize, path: T) -> Result<()> {
        let path = path.as_ref();
        debug!("Extracting zip file to path: {}", path.to_string_lossy());
        let p = PathBuf::from(path);
        let parent = p.parent().unwrap();

        if !parent.exists() {
            debug!("Creating parent directory: {:?}", parent);
            util::retry_io(|| fs::create_dir_all(parent))?;
        }

        let mut archive = self.zip.borrow_mut();
        let mut file = archive.by_index(index)?;
        let mut outfile = util::retry_io(|| File::create(path))?;
        let mut buffer = [0; 64000]; // Use a 64KB buffer; good balance for large/small files.

        debug!("Writing file to disk with 64k buffer: {:?}", path);
        loop {
            let len = file.read(&mut buffer)?;
            if len == 0 {
                break; // End of file
            }
            outfile.write_all(&buffer[..len])?;
        }

        Ok(())
    }

    pub fn extract_zip_predicate_to_path<F, T: AsRef<Path>>(&self, predicate: F, path: T) -> Result<usize>
    where
        F: Fn(&str) -> bool,
    {
        let idx = self.find_zip_file(predicate);
        if idx.is_none() {
            bail!("Could not find file in bundle.");
        }
        let idx = idx.unwrap();
        self.extract_zip_idx_to_path(idx, path)?;
        Ok(idx)
    }

    #[cfg(not(target_os = "linux"))]
    pub fn extract_lib_contents_to_path<P: AsRef<Path>, F: Fn(i16)>(&self, current_path: P, progress: F) -> Result<()> {
        let current_path = current_path.as_ref();
        let files = self.get_file_names()?;
        let num_files = files.len();

        info!("Extracting {} app files...", num_files);
        let re = Regex::new(r"lib[\\\/][^\\\/]*[\\\/]").unwrap();
        let stub_regex = Regex::new("_ExecutionStub.exe$").unwrap();
        let updater_idx = self.find_zip_file(|name| name.ends_with("Squirrel.exe"));

        #[cfg(target_os = "windows")]
        {
            let nuspec_path = current_path.join("sq.version");
            let _ = self
                .extract_zip_predicate_to_path(|name| name.ends_with(".nuspec"), nuspec_path)
                .map_err(|_| anyhow!("This package is missing a nuspec manifest."))?;
        }

        for (i, key) in files.iter().enumerate() {
            if Some(i) == updater_idx || !re.is_match(key) || key.ends_with("/") || key.ends_with("\\") {
                debug!("    {} Skipped '{}'", i, key);
                continue;
            }

            let file_path_in_zip = re.replace(key, "").to_string();
            let file_path_on_disk = Path::new(&current_path).join(&file_path_in_zip);

            if stub_regex.is_match(&file_path_in_zip) {
                // let stub_key = stub_regex.replace(&file_path_in_zip, ".exe").to_string();
                // file_path_on_disk = root_path.join(&stub_key);
                debug!("    {} Skipped Stub (obsolete) '{}'", i, key);
                continue;
            }

            // on windows, the zip paths are / and should be \ instead
            #[cfg(target_os = "windows")]
            let file_path_on_disk = file_path_on_disk.normalize_virtually()?;
            #[cfg(target_os = "windows")]
            let file_path_on_disk = file_path_on_disk.as_path();

            debug!("    {} Extracting '{}' to '{}'", i, key, file_path_on_disk.to_string_lossy());
            self.extract_zip_idx_to_path(i, &file_path_on_disk)?;

            // on macos, we need to chmod +x the executable files
            #[cfg(target_os = "macos")]
            {
                if let Ok(file) = std::fs::OpenOptions::new().read(true).open(&file_path_on_disk) {
                    let buf = std::io::BufReader::new(file);
                    if let Ok(det) = bindet::detect(buf).map_err(|e| e.kind()) {
                        if let Some(matches) = det {
                            for m in matches.likely_to_be {
                                if m == bindet::FileType::Mach {
                                    if let Err(e) = std::fs::set_permissions(&file_path_on_disk, std::fs::Permissions::from_mode(0o755)) {
                                        warn!("Failed to set executable permissions on '{}': {}", file_path_on_disk.to_string_lossy(), e);
                                    } else {
                                        info!("    {} Set executable permissions on '{}'", i, file_path_on_disk.to_string_lossy());
                                    }
                                    break;
                                }
                            }
                        }
                    }
                }
            }

            progress(((i as f32 / num_files as f32) * 100.0) as i16);
        }

        Ok(())
    }

    pub fn read_manifest(&self) -> Result<Manifest> {
        let nuspec_idx = self
            .find_zip_file(|name| name.ends_with(".nuspec"))
            .ok_or_else(|| anyhow!("This installer is missing a package manifest (.nuspec). Please contact the application author."))?;
        let mut contents = String::new();
        let mut archive = self.zip.borrow_mut();
        archive.by_index(nuspec_idx)?.read_to_string(&mut contents)?;
        let app = read_manifest_from_string(&contents)?;
        Ok(app)
    }

    pub fn copy_bundle_to_file<T: AsRef<str>>(&self, nupkg_path: T) -> Result<()> {
        let nupkg_path = nupkg_path.as_ref();
        if self.zip_from_file {
            util::retry_io(|| fs::copy(self.file_path.clone().unwrap(), nupkg_path))?;
        } else {
            util::retry_io(|| fs::write(nupkg_path, self.zip_range.unwrap()))?;
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        let archive = self.zip.borrow();
        archive.len()
    }

    pub fn get_file_names(&self) -> Result<Vec<String>> {
        let mut files: Vec<String> = Vec::new();
        let mut archive = self.zip.borrow_mut();
        for i in 0..archive.len() {
            let file = archive.by_index(i)?;
            let key = file.name();
            files.push(key.to_string());
        }
        Ok(files)
    }
}

#[derive(Debug, derivative::Derivative, Clone)]
#[derivative(Default)]
pub struct Manifest {
    pub id: String,
    #[derivative(Default(value = "Version::new(0, 0, 0)"))]
    pub version: Version,
    pub title: String,
    pub authors: String,
    pub description: String,
    pub machine_architecture: String,
    pub runtime_dependencies: String,
    pub main_exe: String,
    pub os: String,
    pub os_min_version: String,
    pub channel: String,
}

pub fn read_manifest_from_string(xml: &str) -> Result<Manifest> {
    let mut obj: Manifest = Default::default();
    let cursor = Cursor::new(xml);
    let parser = EventReader::new(cursor);
    let mut vec: Vec<String> = Vec::new();
    for e in parser {
        match e {
            Ok(XmlEvent::StartElement { name, .. }) => {
                vec.push(name.local_name);
            }
            Ok(XmlEvent::Characters(text)) => {
                if vec.is_empty() {
                    continue;
                }
                let el_name = vec.last().unwrap();
                if el_name == "id" {
                    obj.id = text;
                } else if el_name == "version" {
                    obj.version = Version::parse(&text)?;
                } else if el_name == "title" {
                    obj.title = text;
                } else if el_name == "authors" {
                    obj.authors = text;
                } else if el_name == "description" {
                    obj.description = text;
                } else if el_name == "machineArchitecture" {
                    obj.machine_architecture = text;
                } else if el_name == "runtimeDependencies" {
                    obj.runtime_dependencies = text;
                } else if el_name == "mainExe" {
                    obj.main_exe = text;
                } else if el_name == "os" {
                    obj.os = text;
                } else if el_name == "osMinVersion" {
                    obj.os_min_version = text;
                } else if el_name == "channel" {
                    obj.channel = text;
                }
            }
            Ok(XmlEvent::EndElement { .. }) => {
                vec.pop();
            }
            Err(e) => {
                error!("Error: {e}");
                break;
            }
            // There's more: https://docs.rs/xml-rs/latest/xml/reader/enum.XmlEvent.html
            _ => {}
        }
    }

    if obj.id.is_empty() {
        bail!("Missing 'id' in package manifest. Please contact the application author.");
    }

    if obj.version == Version::new(0, 0, 0) {
        bail!("Missing 'version' in package manifest. Please contact the application author.");
    }

    #[cfg(target_os = "windows")]
    if obj.main_exe.is_empty() {
        bail!("Missing 'mainExe' in package manifest. Please contact the application author.");
    }

    if obj.title.is_empty() {
        obj.title = obj.id.clone();
    }

    Ok(obj)
}

#[derive(Debug, Clone, derivative::Derivative)]
#[derivative(Default)]
pub struct EntryNameInfo {
    pub name: String,
    #[derivative(Default(value = "Version::new(0, 0, 0)"))]
    pub version: Version,
    pub is_delta: bool,
    pub file_path: String,
}

impl EntryNameInfo {
    pub fn load_manifest(&self) -> Result<Manifest> {
        let path = Path::new(&self.file_path).to_path_buf();
        let bundle = load_bundle_from_file(&path)?;
        bundle.read_manifest()
    }
}

lazy_static! {
    static ref ENTRY_SUFFIX_FULL: Regex = Regex::new(r"(?i)-full.nupkg$").unwrap();
    static ref ENTRY_SUFFIX_DELTA: Regex = Regex::new(r"(?i)-delta.nupkg$").unwrap();
    static ref ENTRY_VERSION_START: Regex = Regex::new(r"[\.-](0|[1-9]\d*)\.(0|[1-9]\d*)($|[^\d])").unwrap();
}

pub fn parse_package_file_path(path: PathBuf) -> Option<EntryNameInfo> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let m = parse_package_file_name(name);
    if m.is_some() {
        let mut m = m.unwrap();
        m.file_path = path.to_string_lossy().to_string();
        return Some(m);
    }
    m
}

fn parse_package_file_name<T: AsRef<str>>(name: T) -> Option<EntryNameInfo> {
    let name = name.as_ref();
    let full = ENTRY_SUFFIX_FULL.is_match(name);
    let delta = ENTRY_SUFFIX_DELTA.is_match(name);
    if !full && !delta {
        return None;
    }

    let mut entry = EntryNameInfo::default();
    entry.is_delta = delta;

    let name_and_ver = if full { ENTRY_SUFFIX_FULL.replace(name, "") } else { ENTRY_SUFFIX_DELTA.replace(name, "") };
    let ver_idx = ENTRY_VERSION_START.find(&name_and_ver);
    if ver_idx.is_none() {
        return None;
    }

    let ver_idx = ver_idx.unwrap().start();
    entry.name = name_and_ver[0..ver_idx].to_string();
    let ver_idx = ver_idx + 1;
    let version = name_and_ver[ver_idx..].to_string();

    let sv = Version::parse(&version);
    if sv.is_err() {
        return None;
    }

    entry.version = sv.unwrap();
    return Some(entry);
}

#[test]
fn test_parse_package_file_name() {
    // test no rid
    let entry = parse_package_file_name("Velopack-1.0.0-full.nupkg").unwrap();
    assert_eq!(entry.name, "Velopack");
    assert_eq!(entry.version, Version::parse("1.0.0").unwrap());
    assert_eq!(entry.is_delta, false);

    let entry = parse_package_file_name("Velopack-1.0.0-delta.nupkg").unwrap();
    assert_eq!(entry.name, "Velopack");
    assert_eq!(entry.version, Version::parse("1.0.0").unwrap());
    assert_eq!(entry.is_delta, true);

    let entry = parse_package_file_name("My.Cool-App-1.1.0-full.nupkg").unwrap();
    assert_eq!(entry.name, "My.Cool-App");
    assert_eq!(entry.version, Version::parse("1.1.0").unwrap());
    assert_eq!(entry.is_delta, false);

    // test invalid names
    assert!(parse_package_file_name("MyCoolApp-1.2.3-beta1-win7-x64-full.nupkg.zip").is_none());
    assert!(parse_package_file_name("MyCoolApp-1.2.3-beta1-win7-x64-full.zip").is_none());
    assert!(parse_package_file_name("MyCoolApp-1.2.3.nupkg").is_none());
    assert!(parse_package_file_name("MyCoolApp-1.2-full.nupkg").is_none());
}
