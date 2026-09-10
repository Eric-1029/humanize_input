using System.Windows;

namespace HumanizeInput.App.Analysis;

public partial class TypingFrequencyDetectorWindow : Window
{
    public TypingFrequencyDetectorWindow(string uiLanguageCode)
    {
        InitializeComponent();
        DataContext = new TypingFrequencyDetectorViewModel(uiLanguageCode);
        Loaded += OnLoaded;
    }

    public TypingFrequencyDetectorViewModel ViewModel => (TypingFrequencyDetectorViewModel)DataContext;

    public TypingFitResult? Result => ViewModel.LatestResult;

    private void OnColumnsSizeChanged(object sender, SizeChangedEventArgs e)
    {
        int columns = e.NewSize.Width >= 740 ? 2 : 1;
        DetectorColumns.ItemWidth = Math.Max(1, e.NewSize.Width / columns);
    }

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        TypingAreaBox.Focus();
        TypingAreaBox.Select(0, 0);
    }

    private void OnCloseClick(object sender, RoutedEventArgs e)
    {
        Close();
    }

    private void OnCaptureButtonClick(object sender, RoutedEventArgs e)
    {
        Dispatcher.BeginInvoke(new Action(() =>
        {
            if (ViewModel.IsRecording)
            {
                TypingAreaBox.Focus();
                TypingAreaBox.CaretIndex = TypingAreaBox.Text.Length;
            }
        }));
    }

    private void OnApplyClick(object sender, RoutedEventArgs e)
    {
        if (!ViewModel.CanApply && !ViewModel.GenerateFit())
        {
            return;
        }

        DialogResult = true;
        Close();
    }
}
