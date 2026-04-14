using System.ComponentModel;
using System.Drawing;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows;
using System.Windows.Interop;
using System.Windows.Input;
using Forms = System.Windows.Forms;
using HumanizeInput.App.Analysis;
using HumanizeInput.App.Settings;
using HumanizeInput.App.ViewModels;
using HumanizeInput.Core;
using HumanizeInput.Infra.Input;

namespace HumanizeInput.App;

public partial class MainWindow : Window
{
    private const int WmHotkey = 0x0312;
    private const int WmNchittest = 0x0084;
    private const int StartHotkeyId = 9001;
    private const int PauseHotkeyId = 9002;
    private const int HtLeft = 10;
    private const int HtRight = 11;
    private const int HtTop = 12;
    private const int HtTopLeft = 13;
    private const int HtTopRight = 14;
    private const int HtBottom = 15;
    private const int HtBottomLeft = 16;
    private const int HtBottomRight = 17;
    private const double ResizeBorderThickness = 8d;

    private readonly MainViewModel _viewModel;
    private HwndSource? _hwndSource;
    private Forms.NotifyIcon? _trayIcon;
    private Forms.ToolStripMenuItem? _openMenuItem;
    private Forms.ToolStripMenuItem? _exitMenuItem;
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
        _viewModel.UiLanguageChanged += UpdateTrayMenuItemsText;
        DataContext = _viewModel;

        Loaded += OnLoaded;
        SourceInitialized += OnSourceInitialized;
        Deactivated += OnDeactivated;
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

    private void OnDeactivated(object? sender, EventArgs e)
    {
        if (_isExitRequested || !IsVisible || !ShowInTaskbar)
        {
            return;
        }

        nint foregroundWindow = GetForegroundWindow();
        if (foregroundWindow != nint.Zero)
        {
            _ = GetWindowThreadProcessId(foregroundWindow, out uint processId);
            if (processId == (uint)Environment.ProcessId)
            {
                return;
            }
        }

        HideToTray();
    }

    private void OnWindowFrameMouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        if (e.ChangedButton != MouseButton.Left || !ReferenceEquals(sender, e.OriginalSource))
        {
            return;
        }

        DragMove();
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
        _openMenuItem = new Forms.ToolStripMenuItem(_viewModel.TrayOpenMenuText);
        _openMenuItem.Click += (_, _) => ShowFromTray();
        _exitMenuItem = new Forms.ToolStripMenuItem(_viewModel.TrayExitMenuText);
        _exitMenuItem.Click += (_, _) => ExitApplication();
        menu.Items.Add(_openMenuItem);
        menu.Items.Add(_exitMenuItem);

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
        SettingsPopup.IsOpen = false;
        WindowState = WindowState.Minimized;
        ShowInTaskbar = false;
        Hide();
    }

    private void OnSettingsMenuButtonClick(object sender, RoutedEventArgs e)
    {
        SettingsPopup.IsOpen = !SettingsPopup.IsOpen;
        e.Handled = true;
    }

    private void OnOpenTypingDetectorClick(object sender, RoutedEventArgs e)
    {
        SettingsPopup.IsOpen = false;

        TypingFrequencyDetectorWindow detectorWindow = new(_viewModel.CurrentLanguage)
        {
            Owner = this
        };

        bool? dialogResult = detectorWindow.ShowDialog();
        if (dialogResult == true && detectorWindow.Result is not null)
        {
            _viewModel.ApplyTypingFitResult(detectorWindow.Result);
        }

        e.Handled = true;
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
        _viewModel.ReportHotkeyRegistrationResult(startOk && pauseOk, startHotkeyText, pauseHotkeyText);
    }

    private void UpdateTrayMenuItemsText()
    {
        Dispatcher.Invoke(() =>
        {
            if (_openMenuItem is not null)
            {
                _openMenuItem.Text = _viewModel.TrayOpenMenuText;
            }

            if (_exitMenuItem is not null)
            {
                _exitMenuItem.Text = _viewModel.TrayExitMenuText;
            }
        });
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
        if (msg == WmNchittest)
        {
            nint resizeHit = HitTestResizeBorder(lParam);
            if (resizeHit != nint.Zero)
            {
                handled = true;
                return resizeHit;
            }

            return nint.Zero;
        }

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

    private nint HitTestResizeBorder(nint lParam)
    {
        if (WindowState == WindowState.Maximized ||
            (ResizeMode is not ResizeMode.CanResize and not ResizeMode.CanResizeWithGrip))
        {
            return nint.Zero;
        }

        int packed = unchecked((int)lParam.ToInt64());
        int screenX = (short)(packed & 0xFFFF);
        int screenY = (short)((packed >> 16) & 0xFFFF);

        System.Windows.Point point = PointFromScreen(new System.Windows.Point(screenX, screenY));
        if (point.X < 0 || point.Y < 0 || point.X > ActualWidth || point.Y > ActualHeight)
        {
            return nint.Zero;
        }

        bool isLeft = point.X <= ResizeBorderThickness;
        bool isRight = point.X >= ActualWidth - ResizeBorderThickness;
        bool isTop = point.Y <= ResizeBorderThickness;
        bool isBottom = point.Y >= ActualHeight - ResizeBorderThickness;

        if (isTop && isLeft)
        {
            return (nint)HtTopLeft;
        }

        if (isTop && isRight)
        {
            return (nint)HtTopRight;
        }

        if (isBottom && isLeft)
        {
            return (nint)HtBottomLeft;
        }

        if (isBottom && isRight)
        {
            return (nint)HtBottomRight;
        }

        if (isLeft)
        {
            return (nint)HtLeft;
        }

        if (isRight)
        {
            return (nint)HtRight;
        }

        if (isTop)
        {
            return (nint)HtTop;
        }

        if (isBottom)
        {
            return (nint)HtBottom;
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

    [DllImport("user32.dll")]
    private static extern uint GetWindowThreadProcessId(nint hWnd, out uint lpdwProcessId);
}
