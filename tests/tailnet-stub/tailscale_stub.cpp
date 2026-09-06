// Cross-platform stub of the `tailscale` CLI for harbor_tailnet_tests.
//
// A shell script cannot stand in here: the adapter resolves `tailscale.exe`
// on Windows, where POSIX scripts do not execute. This tiny dependency-free
// program is built as `tailscale` / `tailscale.exe` by CMake and behaves
// identically on every platform, driven by the same environment protocol
// the shell stub used:
//
//   STUB_LOG    append-only call log (argv echo lines + KEYFILE_CONTENT)
//   STUB_STATE  file containing "logged-in" while the client is logged in
//   STUB_DENY   "permission" | "down" | ""  (failure injection for `up`)
//
// `status` prints a Tailnet address iff the state file says logged in.
// `up` consumes the credential exclusively from a --auth-key=file:PATH
// argument and records the file's content — proving the key never traveled
// in argv — then marks the state logged in.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <string>
#include <vector>

namespace {

std::string getenvOr(const char *name)
{
    const char *value = std::getenv(name);
    return value != nullptr ? value : "";
}

void appendLog(const std::string &line)
{
    const std::string path = getenvOr("STUB_LOG");
    if (path.empty())
        return;
    FILE *file = std::fopen(path.c_str(), "a");
    if (file == nullptr)
        return;
    std::fputs(line.c_str(), file);
    std::fputs("\n", file);
    std::fclose(file);
}

std::string readFile(const std::string &path)
{
    std::ifstream in(path.c_str(), std::ios::binary);
    if (!in)
        return "";
    return std::string((std::istreambuf_iterator<char>(in)),
                       std::istreambuf_iterator<char>());
}

bool fileContains(const std::string &path, const std::string &needle)
{
    const std::string content = readFile(path);
    return !content.empty() && content.find(needle) != std::string::npos;
}

void writeFile(const std::string &path, const std::string &content)
{
    std::ofstream out(path.c_str(), std::ios::binary | std::ios::trunc);
    out << content;
}

} // namespace

int main(int argc, char **argv)
{
    std::vector<std::string> args;
    for (int index = 1; index < argc; ++index)
        args.push_back(argv[index]);
    const std::string command = args.empty() ? "" : args[0];

    // Echo line, same shape as the historical shell stub produced:
    // "<cmd> argv:<args space-joined>". The leak assertion scans exactly
    // these lines for credential material.
    std::string echoed = command + " argv:";
    for (std::vector<std::string>::const_iterator it = args.begin();
         it != args.end(); ++it) {
        echoed += *it;
        echoed += ' ';
    }
    appendLog(echoed);

    if (command == "status") {
        if (fileContains(getenvOr("STUB_STATE"), "logged-in")) {
            std::puts("100.99.99.99  stub-node  tester  linux  -");
            return 0;
        }
        std::fputs("Logged out.\n", stderr);
        return 1;
    }

    if (command == "up") {
        const std::string deny = getenvOr("STUB_DENY");
        if (deny == "permission") {
            std::fputs("Error: tailscaled needs root or operator\n", stderr);
            return 1;
        }
        if (deny == "down") {
            std::fputs("Error: failed to connect to local tailscaled\n", stderr);
            return 1;
        }
        std::string keyFile;
        for (std::vector<std::string>::const_iterator it = args.begin();
             it != args.end(); ++it) {
            const std::string::size_type at = it->find("file:");
            if (at != std::string::npos)
                keyFile = it->substr(at + 5);
        }
        const std::string key = keyFile.empty() ? "" : readFile(keyFile);
        if (key.empty()) {
            std::fputs("Error: no auth key\n", stderr);
            return 1;
        }
        appendLog("up KEYFILE_CONTENT:" + key);
        writeFile(getenvOr("STUB_STATE"), "logged-in");
        return 0;
    }

    std::fputs("stub: unknown command\n", stderr);
    return 1;
}
