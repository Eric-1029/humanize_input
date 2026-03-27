using System.Runtime.InteropServices;
using HumanizeInput.Core;

namespace HumanizeInput.Infra.Input;

public sealed class WindowsSendInputDriver : ITypingDriver
{
    private const uint InputKeyboard = 1;
    private const uint KeyeventfKeyup = 0x0002;
    private const uint KeyeventfUnicode = 0x0004;
    private const ushort VkShift = 0x10;
    private const ushort VkSpace = 0x20;
    private const ushort VkReturn = 0x0D;
    private const ushort VkTab = 0x09;
    private const ushort VkBack = 0x08;

    public nint GetForegroundWindowHandle()
    {
        return GetForegroundWindow();
    }

    public async Task TypeCharAsync(char value, CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();

        if (TryGetVirtualKey(value, out ushort vk, out bool requiresShift))
        {
            try
            {
                if (requiresShift)
                {
                    SendVirtualKeyDown(VkShift);
                }

                SendVirtualKey(vk);

                if (requiresShift)
                {
                    SendVirtualKeyUp(VkShift);
                }

                await Task.Yield();
                return;
            }
            catch (InvalidOperationException)
            {
                // Fall through to Unicode path when a target control rejects VK events.
            }
        }

        INPUT[] inputs =
        [
            CreateUnicodeInput(value, keyup: false),
            CreateUnicodeInput(value, keyup: true)
        ];

        SendInputOrThrow(inputs);
        await Task.Yield();
    }

    public async Task BackspaceAsync(CancellationToken cancellationToken)
    {
        cancellationToken.ThrowIfCancellationRequested();

        try
        {
            INPUT[] inputs =
            [
                new INPUT
                {
                    type = InputKeyboard,
                    U = new InputUnion
                    {
                        ki = new KEYBDINPUT
                        {
                            wVk = VkBack,
                            wScan = 0,
                            dwFlags = 0,
                            time = 0,
                            dwExtraInfo = nint.Zero
                        }
                    }
                },
                new INPUT
                {
                    type = InputKeyboard,
                    U = new InputUnion
                    {
                        ki = new KEYBDINPUT
                        {
                            wVk = VkBack,
                            wScan = 0,
                            dwFlags = KeyeventfKeyup,
                            time = 0,
                            dwExtraInfo = nint.Zero
                        }
                    }
                }
            ];

            SendInputOrThrow(inputs);
        }
        catch (InvalidOperationException)
        {
            INPUT[] unicodeFallback =
            [
                CreateUnicodeInput('\b', keyup: false),
                CreateUnicodeInput('\b', keyup: true)
            ];

            SendInputOrThrow(unicodeFallback);
        }

        await Task.Yield();
    }

    private static INPUT CreateUnicodeInput(char value, bool keyup)
    {
        return new INPUT
        {
            type = InputKeyboard,
            U = new InputUnion
            {
                ki = new KEYBDINPUT
                {
                    wVk = 0,
                    wScan = value,
                    dwFlags = keyup ? KeyeventfUnicode | KeyeventfKeyup : KeyeventfUnicode,
                    time = 0,
                    dwExtraInfo = nint.Zero
                }
            }
        };
    }

    private static void SendInputOrThrow(INPUT[] inputs)
    {
        uint sent = SendInput((uint)inputs.Length, inputs, Marshal.SizeOf<INPUT>());
        if (sent != inputs.Length)
        {
            int errorCode = Marshal.GetLastWin32Error();
            throw new InvalidOperationException($"SendInput failed, sent={sent}, expected={inputs.Length}, win32={errorCode}");
        }
    }

    private static bool TryGetVirtualKey(char value, out ushort vk, out bool shift)
    {
        vk = 0;
        shift = false;

        if (value is >= 'a' and <= 'z')
        {
            vk = (ushort)char.ToUpperInvariant(value);
            return true;
        }

        if (value is >= 'A' and <= 'Z')
        {
            vk = (ushort)value;
            shift = true;
            return true;
        }

        if (value is >= '0' and <= '9')
        {
            vk = (ushort)value;
            return true;
        }

        switch (value)
        {
            case ' ':
                vk = VkSpace;
                return true;
            case '\r':
            case '\n':
                vk = VkReturn;
                return true;
            case '\t':
                vk = VkTab;
                return true;

            case '-': vk = 0xBD; return true;
            case '_': vk = 0xBD; shift = true; return true;
            case '=': vk = 0xBB; return true;
            case '+': vk = 0xBB; shift = true; return true;
            case '[': vk = 0xDB; return true;
            case '{': vk = 0xDB; shift = true; return true;
            case ']': vk = 0xDD; return true;
            case '}': vk = 0xDD; shift = true; return true;
            case '\\': vk = 0xDC; return true;
            case '|': vk = 0xDC; shift = true; return true;
            case ';': vk = 0xBA; return true;
            case ':': vk = 0xBA; shift = true; return true;
            case '\'': vk = 0xDE; return true;
            case '"': vk = 0xDE; shift = true; return true;
            case ',': vk = 0xBC; return true;
            case '<': vk = 0xBC; shift = true; return true;
            case '.': vk = 0xBE; return true;
            case '>': vk = 0xBE; shift = true; return true;
            case '/': vk = 0xBF; return true;
            case '?': vk = 0xBF; shift = true; return true;
            case '`': vk = 0xC0; return true;
            case '~': vk = 0xC0; shift = true; return true;
            case '!': vk = (ushort)'1'; shift = true; return true;
            case '@': vk = (ushort)'2'; shift = true; return true;
            case '#': vk = (ushort)'3'; shift = true; return true;
            case '$': vk = (ushort)'4'; shift = true; return true;
            case '%': vk = (ushort)'5'; shift = true; return true;
            case '^': vk = (ushort)'6'; shift = true; return true;
            case '&': vk = (ushort)'7'; shift = true; return true;
            case '*': vk = (ushort)'8'; shift = true; return true;
            case '(': vk = (ushort)'9'; shift = true; return true;
            case ')': vk = (ushort)'0'; shift = true; return true;
            default:
                return false;
        }
    }

    private static void SendVirtualKey(ushort vk)
    {
        SendVirtualKeyDown(vk);
        SendVirtualKeyUp(vk);
    }

    private static void SendVirtualKeyDown(ushort vk)
    {
        INPUT[] inputs =
        [
            new INPUT
            {
                type = InputKeyboard,
                U = new InputUnion
                {
                    ki = new KEYBDINPUT
                    {
                        wVk = vk,
                        wScan = 0,
                        dwFlags = 0,
                        time = 0,
                        dwExtraInfo = nint.Zero
                    }
                }
            }
        ];

        SendInputOrThrow(inputs);
    }

    private static void SendVirtualKeyUp(ushort vk)
    {
        INPUT[] inputs =
        [
            new INPUT
            {
                type = InputKeyboard,
                U = new InputUnion
                {
                    ki = new KEYBDINPUT
                    {
                        wVk = vk,
                        wScan = 0,
                        dwFlags = KeyeventfKeyup,
                        time = 0,
                        dwExtraInfo = nint.Zero
                    }
                }
            }
        ];

        SendInputOrThrow(inputs);
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct INPUT
    {
        public uint type;
        public InputUnion U;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct InputUnion
    {
        [FieldOffset(0)]
        public MOUSEINPUT mi;

        [FieldOffset(0)]
        public KEYBDINPUT ki;

        [FieldOffset(0)]
        public HARDWAREINPUT hi;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MOUSEINPUT
    {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint dwFlags;
        public uint time;
        public nint dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HARDWAREINPUT
    {
        public uint uMsg;
        public ushort wParamL;
        public ushort wParamH;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct KEYBDINPUT
    {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public nint dwExtraInfo;
    }

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint nInputs, INPUT[] pInputs, int cbSize);

    [DllImport("user32.dll")]
    private static extern nint GetForegroundWindow();
}
