# humanize_input

Windows 原生逐字输入模拟工具（C# + WPF）。

English version: [README.md](README.md)

## 已实现功能

- 粘贴文本后逐字输入
- 输入速度与抖动调节
- 错字/漏写/相邻字符颠倒概率模拟
- 邻键错字（基于键盘邻近位置，距离越近概率越高）
- 修复率调节（错误不一定都会被修复）
- 延迟修复模拟（部分情况下会先继续输入 1-2 个字符再回看修复）
- 数字字符不参与错字模拟
- CJK 字符输入（Unicode 路径）与常见 ASCII 键盘路径的混合注入
- 全局热键开始、暂停/继续（支持修改）
- 程序启动后自动最小化到托盘，双击托盘图标恢复窗口
- 焦点窗口变更时自动等待，焦点恢复后继续输入
- 窗口按比例缩放（非最大化时也能完整看到参数区）
- 文/A 双语界面切换（中英），设置项与介绍文案均可翻译
- 输入频率检测器：录入样本并拟合模拟参数
- 设置自动持久化到 INI 文件
- 启动自动加载、修改后自动保存
- 首次启动自动生成默认 `settings.ini`

## 项目结构

- src/HumanizeInput.App: WPF 界面与 ViewModel
- src/HumanizeInput.Core: 输入会话与随机化逻辑
- src/HumanizeInput.Infra: Windows SendInput 注入实现
- tests/HumanizeInput.Core.Tests: 核心单元测试

## 运行前提

请先安装 .NET 8 SDK 与 Windows Desktop Runtime。

## 直接运行 Release

如果你下载的是 release 的 ZIP 包，例如 `dist/humanize_input-v1.1.1-win-x64.zip`，解压后直接双击 `HumanizeInput.App.exe` 即可启动，不需要先编译。启动后程序会自动最小化到托盘；如果你只看到托盘图标或小窗，这是正常现象，说明程序已经启动。需要时双击托盘图标可恢复主窗口。

## 构建与运行

```powershell
dotnet build .\humanize_input.sln
dotnet run --project .\src\HumanizeInput.App\HumanizeInput.App.csproj
```

## 使用流程

1. 在文本框粘贴内容
2. 调整输入参数（速度、抖动、错字率、漏写率、颠倒率、修复率）
3. 配置并应用全局热键（默认开始 `Ctrl+Alt+S`，暂停/继续 `Ctrl+Alt+P`）
4. 如果想根据真实输入样本拟合参数，可在设置面板里打开输入检测器
5. 程序会自动最小化到托盘，双击托盘图标可恢复窗口
6. 先把目标编辑器光标放在可输入区域，再按开始热键
7. 输入过程中按暂停热键可暂停/继续

## 配置文件（INI）

- 默认位置：与程序可执行文件同目录（`settings.ini`）。
- 首次启动会自动写入默认配置。
- 用户调整设置项后会自动保存。
- 下次启动会自动从 INI 读取并恢复配置。
- 语言会通过 `language=zh-CN|en-US` 持久化。

## 参数建议（更像人类）

- 基础延迟: 70-140 ms
- 抖动: 15%-35%
- 错字率: 5%-10%
- 漏写率: 2%-6%
- 颠倒率: 2%-5%
- 修复率: 70%-90%

## 注意事项

- 遵守 Windows 输入模型：输入发送到前台焦点窗口。
- 如果目标应用以管理员权限运行，本程序也应以相同权限运行。
- 若发生 DLL 被占用导致构建警告/失败，先关闭正在运行的 humanize_input 再构建。
