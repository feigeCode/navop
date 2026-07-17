@echo off
setlocal EnableExtensions

rem Navop run-from-source script
rem Usage:
rem   run.bat
rem   run.bat --release
rem   run.bat -- <args>

cd /d "%~dp0"
if errorlevel 1 (
    echo [ERROR] Failed to change directory to project root: %~dp0
    exit /b 1
)

where cargo >nul 2>nul
if errorlevel 1 (
    echo [ERROR] cargo not found. Install Rust and add cargo to PATH.
    echo         https://rustup.rs
    exit /b 1
)

where rustup >nul 2>nul
if not errorlevel 1 (
    if exist "%CD%\rust-toolchain.toml" (
        echo [info] Ensuring project toolchain from rust-toolchain.toml ...
        rustup show active-toolchain
        if errorlevel 1 (
            echo [ERROR] Failed to activate project toolchain.
            echo         Try: rustup install 1.95.0
            exit /b 1
        )
    )
)

set "RELEASE_FLAG="
set "APP_ARGS="

:parse_args
if "%~1"=="" goto run
if /I "%~1"=="--release" (
    set "RELEASE_FLAG=--release"
    shift
    goto parse_args
)
if "%~1"=="--" (
    shift
    goto collect_app_args
)
set "APP_ARGS=%APP_ARGS% %1"
shift
goto parse_args

:collect_app_args
if "%~1"=="" goto run
set "APP_ARGS=%APP_ARGS% %1"
shift
goto collect_app_args

:run
echo ========================================
echo  Run Navop from source
echo  Dir: %CD%
if defined RELEASE_FLAG (
    echo  Mode: release
) else (
    echo  Mode: debug
)
echo ========================================
echo.

if defined RELEASE_FLAG (
    cargo run -p main --release --%APP_ARGS%
) else (
    cargo run -p main --%APP_ARGS%
)

set "EXIT_CODE=%ERRORLEVEL%"
if not "%EXIT_CODE%"=="0" (
    echo.
    echo [FAILED] Exit code: %EXIT_CODE%
    echo.
    echo If the error mentions cold_path or E0658, your Rust is too old.
    echo This project needs Rust 1.95.0+.
    echo   rustup install 1.95.0
    echo   rustup show
    exit /b %EXIT_CODE%
)

exit /b 0
