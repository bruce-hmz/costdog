@echo off
echo Packaging CostDog for distribution...
echo.

REM 创建分发目录
if not exist "dist-package" mkdir dist-package

REM 复制必要文件
echo Copying files...
copy start.bat dist-package\
copy package.json dist-package\
copy tsconfig.json dist-package\
xcopy /E /I src dist-package\src
xcopy /E /I node_modules dist-package\node_modules 2>nul

REM 创建启动说明
echo Creating README...
(
echo # CostDog - Cost Monitor for Claude Code
echo.
echo ## Quick Start
echo.
echo 1. Make sure Node.js is installed ^(https://nodejs.org/^)
echo 2. Double-click `start.bat` to launch
echo.
echo ## Manual Start
echo.
echo ```bash
echo npm install
echo npm run dev:web
echo # In another terminal:
echo npm run tauri:dev
echo ```
echo.
echo ## Features
echo.
echo - Real-time cost monitoring
echo - Token usage tracking
echo - Session statistics
echo - Model usage breakdown
) > dist-package\README.md

echo.
echo Package created in: dist-package\
echo.
echo To distribute:
echo 1. Zip the 'dist-package' folder
echo 2. Send to recipient
echo 3. They need Node.js installed
echo 4. They run 'start.bat'
echo.
pause
