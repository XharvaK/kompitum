@echo off
rem Stands in for zig when only type-checking another target (clippy, check),
rem as zig-stub.sh does on Unix: leaves an empty libghostty-vt where the build
rem script looks. Nothing links.
rem   set ZIG=%CD%\scripts\zig-stub.cmd
rem   cargo clippy --locked --target x86_64-unknown-linux-musl --all-targets
setlocal
set next=
for %%a in (%*) do (
    if defined next (
        mkdir "%%~a\lib" 2>nul
        type nul > "%%~a\lib\ghostty-vt-static.lib"
        type nul > "%%~a\lib\libghostty-vt.a"
        set next=
    )
    if "%%~a"=="--prefix" set next=1
)
exit /b 0
