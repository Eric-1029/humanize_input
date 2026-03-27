namespace HumanizeInput.Core;

public interface ITypingDriver
{
    nint GetForegroundWindowHandle();
    Task TypeCharAsync(char value, CancellationToken cancellationToken);
    Task BackspaceAsync(CancellationToken cancellationToken);
}
