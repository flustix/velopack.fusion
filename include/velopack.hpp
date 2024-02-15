#ifndef VELOPACK_H_INCLUDED
#define VELOPACK_H_INCLUDED

#include <string>

namespace Velopack {

#if UNICODE
void startup(wchar_t** args, size_t c_args);
#endif // UNICODE
void startup(char** args, size_t c_args);
bool util_does_file_exist(std::string file_path);
std::string util_current_os_name();
std::string util_get_own_exe_path();
std::string util_string_to_lower(std::string str);
bool ci_equal(const std::string& a, const std::string& b);
// std::string velo_get_own_exe_path();

}
#endif // VELOPACK_H_INCLUDED