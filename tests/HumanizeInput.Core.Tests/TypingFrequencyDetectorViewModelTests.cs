using HumanizeInput.App.Analysis;
using Xunit;

namespace HumanizeInput.Core.Tests;

public class TypingFrequencyDetectorViewModelTests
{
    [Fact]
    public void ToggleCapture_StartsAndStopsRecordingAndGeneratesResult()
    {
        TypingFrequencyDetectorViewModel viewModel = new("en-US");

        Assert.False(viewModel.IsRecording);
        Assert.Equal("Start", viewModel.CaptureButtonText);

        viewModel.ToggleCapture();

        Assert.True(viewModel.IsRecording);
        Assert.Equal("Stop", viewModel.CaptureButtonText);

        viewModel.TypedText = viewModel.PromptText;
        viewModel.ToggleCapture();

        Assert.False(viewModel.IsRecording);
        Assert.Equal("Start", viewModel.CaptureButtonText);
        Assert.NotNull(viewModel.LatestResult);
        Assert.True(viewModel.CanApply);
        Assert.Equal(100.0, Math.Round(viewModel.LatestResult!.AccuracyPercent, 1));
    }

    [Fact]
    public void ToggleCapture_RecognizesAdjacentTransposition()
    {
        TypingFrequencyDetectorViewModel viewModel = new("en-US");

        string promptText = viewModel.PromptText;
        string swappedText = promptText.Length >= 2
            ? string.Concat(promptText[1], promptText[0], promptText[2..])
            : promptText;

        viewModel.ToggleCapture();
        viewModel.TypedText = swappedText;
        viewModel.ToggleCapture();

        Assert.NotNull(viewModel.LatestResult);
        Assert.True(viewModel.LatestResult!.TransposeRatePercent > 0);
        Assert.True(viewModel.LatestResult.AccuracyPercent < 100.0);
    }
}
