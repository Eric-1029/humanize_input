using System.Globalization;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;

namespace HumanizeInput.App.Controls;

public partial class ParameterEditor : System.Windows.Controls.UserControl
{
    public static readonly DependencyProperty LabelProperty = DependencyProperty.Register(nameof(Label), typeof(string), typeof(ParameterEditor), new PropertyMetadata(""));
    public static readonly DependencyProperty MinimumProperty = DependencyProperty.Register(nameof(Minimum), typeof(int), typeof(ParameterEditor), new PropertyMetadata(0));
    public static readonly DependencyProperty MaximumProperty = DependencyProperty.Register(nameof(Maximum), typeof(int), typeof(ParameterEditor), new PropertyMetadata(100));
    public static readonly DependencyProperty RangeTextProperty = DependencyProperty.Register(nameof(RangeText), typeof(string), typeof(ParameterEditor), new PropertyMetadata(""));
    public static readonly DependencyProperty ValueProperty = DependencyProperty.Register(nameof(Value), typeof(int), typeof(ParameterEditor),
        new FrameworkPropertyMetadata(0, FrameworkPropertyMetadataOptions.BindsTwoWayByDefault, OnValueChanged));

    public string Label { get => (string)GetValue(LabelProperty); set => SetValue(LabelProperty, value); }
    public int Minimum { get => (int)GetValue(MinimumProperty); set => SetValue(MinimumProperty, value); }
    public int Maximum { get => (int)GetValue(MaximumProperty); set => SetValue(MaximumProperty, value); }
    public string RangeText { get => (string)GetValue(RangeTextProperty); set => SetValue(RangeTextProperty, value); }
    public int Value { get => (int)GetValue(ValueProperty); set => SetValue(ValueProperty, value); }

    public ParameterEditor()
    {
        InitializeComponent();
        Loaded += (_, _) => RefreshNumber();
    }

    private static void OnValueChanged(DependencyObject sender, DependencyPropertyChangedEventArgs args)
        => ((ParameterEditor)sender).RefreshNumber();

    private void RefreshNumber()
    {
        if (NumberBox is not null)
            NumberBox.Text = Value.ToString(CultureInfo.CurrentCulture);
    }

    private void Commit()
    {
        if (int.TryParse(NumberBox.Text, NumberStyles.Integer, CultureInfo.CurrentCulture, out int number))
            SetCurrentValue(ValueProperty, Math.Clamp(number, Minimum, Maximum));
        RefreshNumber();
    }

    private void OnCommit(object sender, KeyboardFocusChangedEventArgs e) => Commit();

    private void OnNumberBoxGotKeyboardFocus(object sender, KeyboardFocusChangedEventArgs e)
    {
        NumberBox.SelectAll();
    }

    private void OnNumberBoxPreviewMouseLeftButtonDown(object sender, MouseButtonEventArgs e)
    {
        NumberBox.Focus();
        NumberBox.SelectAll();
        e.Handled = true;
    }

    private void OnKeyDown(object sender, System.Windows.Input.KeyEventArgs e)
    {
        if (e.Key == Key.Enter) { Commit(); NumberBox.SelectAll(); e.Handled = true; }
        else if (e.Key == Key.Escape) { RefreshNumber(); NumberBox.SelectAll(); e.Handled = true; }
    }
}
