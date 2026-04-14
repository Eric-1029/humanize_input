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

    private void OnLoaded(object sender, RoutedEventArgs e)
    {
        TypingAreaBox.Focus();
        TypingAreaBox.Select(0, 0);
    }

    private void OnCloseClick(object sender, RoutedEventArgs e)
    {
        Close();
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
