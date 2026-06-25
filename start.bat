@echo off
echo Starting CostDog...
echo.

REM 检查是否安装了 Node.js
where node >nul 2>nul
if %errorlevel% neq 0 (
    echo Error: Node.js is not installed.
    echo Please install Node.js from https://nodejs.org/
    pause
    exit /b 1
)

REM 检查是否安装了依赖
if not exist "node_modules" (
    echo Installing dependencies...
    npm install
    echo.
)

REM 启动 web 服务器（后台）
echo Starting web server...
start /B npm run dev:web

REM 等待服务器启动
timeout /t 3 /nobreak >nul

REM 启动 Tauri 应用
echo Starting CostDog application...
npm run tauri:dev

pause
