#include "my_application.h"

#include <cstring>

int main(int argc, char** argv) {
  if (argc == 2 && std::strcmp(argv[1], "--version") == 0) {
    g_print("Mica %s\n", MICA_APP_VERSION);
    return 0;
  }
  g_autoptr(MyApplication) app = my_application_new();
  return g_application_run(G_APPLICATION(app), argc, argv);
}
