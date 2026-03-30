using System.ComponentModel;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Input;
using Forms = System.Windows.Forms;
using HumanizeInput.App.Settings;
using HumanizeInput.App.ViewModels;
using HumanizeInput.Core;
using HumanizeInput.Infra.Input;

namespace HumanizeInput.App;

public partial class MainWindow : Window
{
    private const int WmHotkey = 0x0312;
    private const int StartHotkeyId = 9001;
    private const int PauseHotkeyId = 9002;

    private readonly MainViewModel _viewModel;
    private HwndSource? _hwndSource;
    private Forms.NotifyIcon? _trayIcon;
    private bool _isExitRequested;

    public MainWindow()
    {
        InitializeComponent();

        ITypingDriver driver = new WindowsSendInputDriver();
        TypingSessionService sessionService = new(driver);
        string settingsPath = Path.Combine(AppContext.BaseDirectory, "settings.ini");
        IniSettingsStore settingsStore = new(settingsPath);

        _viewModel = new MainViewModel(sessionService, driver, settingsStore);
        _viewModel.HotkeyUpdateRequested += RegisterHotkeys;
        DataContext = _viewModel;

        Loaded += OnLoaded;
        SourceInitialized += OnSourceInitialized;
        Closing += OnClosing;
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        EnsureTrayIcon();
        RegisterHotkeys(_viewModel.StartHotkeyText, _viewModel.PauseHotkeyText);
        HideToTray();
    }

    private void OnSourceInitialized(object? sender, EventArgs e)
    {
        nint handle = new WindowInteropHelper(this).Handle;
        _hwndSource = HwndSource.FromHwnd(handle);
        _hwndSource?.AddHook(WndProc);
    }

    private void OnClosing(object? sender, CancelEventArgs e)
    {
        if (!_isExitRequested)
        {
            e.Cancel = true;
            HideToTray();
            return;
        }

        UnregisterAllHotkeys();

        if (_hwndSource is not null)
        {
            _hwndSource.RemoveHook(WndProc);
        }

        if (_trayIcon is not null)
        {
            _trayIcon.Visible = false;
            _trayIcon.Dispose();
        }

        _viewModel.FlushSettings();
    }

    private void EnsureTrayIcon()
    {
        if (_trayIcon is not null)
        {
            return;
        }

        Forms.ContextMenuStrip menu = new();
        menu.Items.Add("打开主窗口", null, (_, _) => ShowFromTray());
        menu.Items.Add("退出", null, (_, _) => ExitApplication());

        _trayIcon = new Forms.NotifyIcon
        {
            Icon = SystemIcons.Application,
            Text = "humanize_input",
            Visible = true,
            ContextMenuStrip = menu
        };
        _trayIcon.DoubleClick += (_, _) => ShowFromTray();
    }

    private void HideToTray()
    {
        WindowState = WindowState.Minimized;
        ShowInTaskbar = false;
        Hide();
    }

    private void ShowFromTray()
    {
        ShowInTaskbar = true;
        Show();
        WindowState = WindowState.Normal;
        Activate();
    }

    private void ExitApplication()
    {
        _isExitRequested = true;
        Close();
    }

    private void RegisterHotkeys(string startHotkeyText, string pauseHotkeyText)
    {
        nint handle = new WindowInteropHelper(this).Handle;
        if (handle == nint.Zero)
        {
            return;
        }

        UnregisterHotKey(handle, StartHotkeyId);
        UnregisterHotKey(handle, PauseHotkeyId);

        bool startOk = TryRegisterOne(handle, StartHotkeyId, startHotkeyText);
        bool pauseOk = TryRegisterOne(handle, PauseHotkeyId, pauseHotkeyText);

        if (startOk && pauseOk)
        {
            _viewModel.StatusText = $"状态: 全局热键已生效（开始: {startHotkeyText}，暂停: {pauseHotkeyText}）";
            return;
        }

        _viewModel.StatusText = "状态: 热键注册失败，请检查格式或避免与系统热键冲突";
    }

    private static bool TryRegisterOne(nint handle, int hotkeyId, string hotkeyText)
    {
        if (!TryParseHotkey(hotkeyText, out uint modifiers, out uint key))
        {
            return false;
        }

        return RegisterHotKey(handle, hotkeyId, modifiers, key);
    }

    private void UnregisterAllHotkeys()
    {
        nint handle = new WindowInteropHelper(this).Handle;
        if (handle == nint.Zero)
        {
            return;
        }

        UnregisterHotKey(handle, StartHotkeyId);
        UnregisterHotKey(handle, PauseHotkeyId);
    }

    private nint WndProc(nint hwnd, int msg, nint wParam, nint lParam, ref bool handled)
    {
        if (msg != WmHotkey)
        {
            return nint.Zero;
        }

        int id = wParam.ToInt32();
        if (id == StartHotkeyId)
        {
            nint currentForeground = GetForegroundWindow();
            _viewModel.TriggerStartHotkey(currentForeground);
            handled = true;
        }
        else if (id == PauseHotkeyId)
        {
            _viewModel.TriggerPauseResumeHotkey();
            handled = true;
        }

        return nint.Zero;
    }

    private static bool TryParseHotkey(string raw, out uint modifiers, out uint key)
    {
        modifiers = 0;
        key = 0;

        if (string.IsNullOrWhiteSpace(raw))
        {
            return false;
        }

        string[] parts = raw.Split('+', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries);
        if (parts.Length < 2)
        {
            return false;
        }

        for (int i = 0; i < parts.Length - 1; i++)
        {
            string token = parts[i].ToLowerInvariant();
            if (token is "ctrl" or "control")
            {
                modifiers |= 0x0002;
            }
            else if (token == "alt")
            {
                modifiers |= 0x0001;
            }
            else if (token == "shift")
            {
                modifiers |= 0x0004;
            }
            else if (token is "win" or "windows")
            {
                modifiers |= 0x0008;
            }
            else
            {
                return false;
            }
        }

        string keyToken = parts[^1];
        if (!Enum.TryParse(keyToken, true, out System.Windows.Input.Key wpfKey))
        {
            return false;
        }

        int vk = KeyInterop.VirtualKeyFromKey(wpfKey);
        if (vk <= 0)
        {
            return false;
        }

        key = (uint)vk;
        return modifiers != 0;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool RegisterHotKey(nint hWnd, int id, uint fsModifiers, uint vk);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern bool UnregisterHotKey(nint hWnd, int id);

    [DllImport("user32.dll")]
    private static extern nint GetForegroundWindow();
}
