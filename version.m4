dnl The kernel version string, which version.c.in expands into the boot
dnl banner.  rust/src/config.rs holds the same string for the version
dnl userspace reads, and changes in the same commit as this file.
dnl
dnl Calendar versioning, YYYY.MINOR: the year, then a counter that
dnl restarts at 1 each January.  A prerelease takes a ~alphaN, ~betaN
dnl or ~rcN suffix, which dpkg sorts before the release itself.
m4_define([AC_PACKAGE_NAME],[WIP Mach])
m4_define([AC_PACKAGE_VERSION],[2026.1~alpha1])
m4_define([AC_PACKAGE_BUGREPORT],[bug-hurd@gnu.org])
m4_define([AC_PACKAGE_TARNAME],[gnumach])
