*Applies to: Windows, MacOS, Linux*

# Getting Started: C++

1. Copy [Velopack.hpp](https://github.com/velopack/velopack.fusion/blob/master/for-cpp/Velopack.hpp) and [Velopack.cpp](https://github.com/velopack/velopack.fusion/blob/master/for-cpp/Velopack.cpp) into your project. 

2. If you are on Linux or MacOS, you'll need [ICU4C](https://icu.unicode.org/) installed. Like many other C libraries, this is bundled into one package. 
Installing the development package with your package manager (`apt` etc) is sufficient to make it available to compilers. The specific command you'll need is OS/Distro specific.

3. Add the `Velopack::startup()` to your entry point (eg. `main()` or `wmain()`) as early as possible, ideally the first statement to run:
```cpp
#include "Velopack.hpp"

wmain(int argc**, wchar_t *argv[ ], wchar_t *envp[ ])
{
    // velopack may exit / restart your app at this statement
    Velopack::startup(argv, argc);

    // ... your other startup code here
}
```

3. Add auto-updates somewhere to your app:
```cpp
#include "Velopack.hpp"
#include <memory>

static void update_app()
{
    Velopack::UpdateManagerSync manager{};
    manager.setUrlOrPath("https://the.place/you-host/updates");

    std::shared_ptr<Velopack::UpdateInfo> updInfo = manager.checkForUpdates();
    if (updInfo == nullptr) {
        return; // no updates available
    }

    manager.downloadUpdates(updInfo->targetFullRelease.get());
    manager.applyUpdatesAndRestart(updInfo->targetFullRelease.get());
}
```

4. Install the `vpk` command line tool:
```sh
dotnet tool update -g vpk
```
***Note: you must have the .NET Core SDK 6 installed to use and update `vpk`***

5. Compile your app to a program using your usual compiler (eg. msvc, cmake, gcc, etc)

6. Copy `Vfusion.exe`, `VfusionMac` or `VfusionNix` to your build output folder. This is a manual step for now, but may be automated in the future. You can compile this yourself, download a [recent build artifact](https://github.com/velopack/velopack.fusion/actions), or grab the latest [npm release](https://www.npmjs.com/package/velopack?activeTab=code) which also bundles the binaries.

>[!WARNING]
>Until this is automated, failing to copy the fusion binary to your update directory will result in your app being unable to update.

7. Package your Velopack release / installers:
```sh
vpk pack -u MyAppUniqueId -v 1.0.0 -p /myBuildDir -e myexename.exe
```

✅ You're Done! Your app now has auto-updates and an installer.
You can upload your release to your website, or use the `vpk upload` command to publish it to the destination of your choice.

Read the [Velopack Documentation](https://velopack.io/docs) or the [Velopack C++ Reference](https://velopack.io/ref/cpp/) for more information.