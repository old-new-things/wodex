#include "wodex_fs.h"

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <limits>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

namespace {

int32_t fs_open(const char *path, uint32_t mode) {
  int flags = 0;
  switch (mode) {
  case WODEX_FS_READ:
    flags = O_RDONLY;
#ifdef O_NOFOLLOW
    flags |= O_NOFOLLOW;
#endif
    break;
  case WODEX_FS_CREATE_NEW:
    flags = O_WRONLY | O_CREAT | O_EXCL;
#ifdef O_NOFOLLOW
    flags |= O_NOFOLLOW;
#endif
    break;
  case WODEX_FS_DIRECTORY:
    flags = O_RDONLY;
#ifdef O_DIRECTORY
    flags |= O_DIRECTORY;
#endif
    break;
  case WODEX_FS_READ_FOLLOW:
    flags = O_RDONLY;
    break;
  default:
    errno = EINVAL;
    return -1;
  }
  return ::open(path, flags, 0600);
}

int64_t fs_read(int32_t fd, uint8_t *data, size_t size) {
  return static_cast<int64_t>(::read(fd, data, size));
}

int64_t fs_write(int32_t fd, const uint8_t *data, size_t size) {
  return static_cast<int64_t>(::write(fd, data, size));
}

int32_t fs_seek(int32_t fd, uint64_t offset) {
  if (offset > static_cast<uint64_t>(std::numeric_limits<off_t>::max())) {
    errno = EOVERFLOW;
    return -1;
  }
  return ::lseek(fd, static_cast<off_t>(offset), SEEK_SET) < 0 ? -1 : 0;
}

int32_t fs_close(int32_t fd) { return ::close(fd); }
int32_t fs_sync(int32_t fd) { return ::fsync(fd); }

void fs_info(const struct stat &status, WodexFileInfo *output) {
  output->kind = S_ISREG(status.st_mode) ? WODEX_FS_REGULAR :
    S_ISDIR(status.st_mode) ? WODEX_FS_DIRECTORY_KIND :
    S_ISLNK(status.st_mode) ? WODEX_FS_SYMLINK : WODEX_FS_OTHER;
  output->size = status.st_size < 0 ? UINT64_MAX : static_cast<uint64_t>(status.st_size);
}

int32_t fs_fstat(int32_t fd, WodexFileInfo *output) {
  struct stat status{};
  if (::fstat(fd, &status) != 0) return -1;
  fs_info(status, output);
  return 0;
}

int32_t fs_stat(const char *path, bool follow, WodexFileInfo *output) {
  struct stat status{};
  if ((follow ? ::stat(path, &status) : ::lstat(path, &status)) != 0) return -1;
  fs_info(status, output);
  return 0;
}

int32_t fs_rename(const char *from, const char *to) { return ::rename(from, to); }
int32_t fs_unlink(const char *path) { return ::unlink(path); }
int32_t fs_mkdir(const char *path) { return ::mkdir(path, 0700); }
int32_t fs_chmod(const char *path, uint32_t mode) { return ::chmod(path, mode); }
int32_t fs_fchmod(int32_t fd, uint32_t mode) { return ::fchmod(fd, mode); }

uint32_t fs_error_kind() {
  switch (errno) {
  case ENOENT: return WODEX_FS_MISSING;
  case EEXIST: return WODEX_FS_EXISTS;
  case EINTR: return WODEX_FS_INTERRUPTED;
  default: return WODEX_FS_ERROR;
  }
}

const char *fs_error_message() { return std::strerror(errno); }
const char *fs_getenv(const char *name) { return std::getenv(name); }
uint64_t fs_pid() { return static_cast<uint64_t>(::getpid()); }

} // namespace

extern "C" WodexFsApi wodex_native_fs_api() {
  return {
#ifdef __SWITCH__
    true,
#else
    false,
#endif
#ifdef __APPLE__
    true,
#else
    false,
#endif
    &fs_open, &fs_read, &fs_write, &fs_seek, &fs_close, &fs_sync,
    &fs_fstat, &fs_stat, &fs_rename, &fs_unlink, &fs_mkdir, &fs_chmod,
    &fs_fchmod, &fs_error_kind, &fs_error_message, &fs_getenv, &fs_pid
  };
}
