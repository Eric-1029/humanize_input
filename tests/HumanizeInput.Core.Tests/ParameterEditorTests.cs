using System.Runtime.ExceptionServices;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using HumanizeInput.App.Controls;
using Xunit;

namespace HumanizeInput.Core.Tests;

public class ParameterEditorTests
{
    [Theory]
    [InlineData("137", 137)]
    [InlineData("999", 500)]
    [InlineData("-1", 20)]
    [InlineData("", 90)]
    [InlineData("abc", 90)]
    [InlineData("99999999999999999999", 90)]
    public void NumericEdit_CommitsValidIntegerOrRestoresPreviousValue(string text, int expected)
    {
        RunSta(() =>
        {
            ParameterEditor editor = new() { Minimum = 20, Maximum = 500, Value = 90 };
            TextBox box = (TextBox)editor.FindName("NumberBox");
            box.Text = text;
            box.RaiseEvent(new KeyboardFocusChangedEventArgs(Keyboard.PrimaryDevice, 0, box, null)
            {
                RoutedEvent = Keyboard.LostKeyboardFocusEvent
            });
            Assert.Equal(expected, editor.Value);
            Assert.Equal(expected.ToString(), box.Text);
            Assert.Equal(expected, ((Slider)editor.FindName("ValueSlider")).Value);
        });
    }

    [Fact]
    public void SliderEdit_UpdatesNumericBox()
    {
        RunSta(() =>
        {
            ParameterEditor editor = new() { Minimum = 20, Maximum = 500, Value = 90 };
            ((Slider)editor.FindName("ValueSlider")).Value = 231;
            Assert.Equal(231, editor.Value);
            Assert.Equal("231", ((TextBox)editor.FindName("NumberBox")).Text);
        });
    }

    [Fact]
    public void NumberBox_IsPlacedToTheRightWithoutOverlappingSlider()
    {
        RunSta(() =>
        {
            ParameterEditor editor = new() { Minimum = 20, Maximum = 500, Value = 90 };
            editor.Measure(new Size(300, double.PositiveInfinity));
            editor.Arrange(new Rect(0, 0, 300, editor.DesiredSize.Height));
            editor.UpdateLayout();

            Slider slider = (Slider)editor.FindName("ValueSlider");
            TextBox box = (TextBox)editor.FindName("NumberBox");
            Point sliderOrigin = slider.TranslatePoint(new Point(), editor);
            Point boxOrigin = box.TranslatePoint(new Point(), editor);

            Assert.Equal(0, Grid.GetColumn(slider));
            Assert.Equal(1, Grid.GetColumn(box));
            Assert.True(sliderOrigin.X + slider.ActualWidth <= boxOrigin.X);
        });
    }

    [Fact]
    public void NumberBox_SelectsAllTextWhenItReceivesKeyboardFocus()
    {
        RunSta(() =>
        {
            ParameterEditor editor = new() { Minimum = 20, Maximum = 500, Value = 90 };
            TextBox box = (TextBox)editor.FindName("NumberBox");
            box.CaretIndex = box.Text.Length;
            box.RaiseEvent(new KeyboardFocusChangedEventArgs(Keyboard.PrimaryDevice, 0, null, box)
            {
                RoutedEvent = Keyboard.GotKeyboardFocusEvent
            });

            Assert.Equal(0, box.SelectionStart);
            Assert.Equal(box.Text.Length, box.SelectionLength);
        });
    }

    [Fact]
    public void SliderTemplate_RendersTrackAndThumbWithUsableSize()
    {
        RunSta(() =>
        {
            ParameterEditor editor = new() { Minimum = 20, Maximum = 500, Value = 90 };
            editor.Measure(new Size(300, double.PositiveInfinity));
            editor.Arrange(new Rect(0, 0, 300, editor.DesiredSize.Height));
            editor.UpdateLayout();

            Slider slider = (Slider)editor.FindName("ValueSlider");
            slider.ApplyTemplate();

            Track track = Assert.IsType<Track>(slider.Template.FindName("PART_Track", slider));
            Thumb thumb = Assert.IsType<Thumb>(slider.Template.FindName("SliderThumb", slider));
            Assert.True(track.ActualWidth > 80);
            Assert.Equal(18, thumb.ActualWidth);
            Assert.Equal(18, thumb.ActualHeight);
        });
    }

    [Theory]
    [InlineData(260)]
    [InlineData(380)]
    public void NarrowEditor_KeepsSliderAndNumberVisible(double width)
    {
        RunSta(() =>
        {
            ParameterEditor editor = new()
            {
                Label = "发现错误后停顿 / Error detect delay (ms)",
                Minimum = 80, Maximum = 3000, Value = 900, RangeText = "80 – 3000 ms"
            };
            editor.Measure(new Size(width, double.PositiveInfinity));
            editor.Arrange(new Rect(0, 0, width, editor.DesiredSize.Height));
            editor.UpdateLayout();
            foreach (string name in new[] { "NumberBox", "ValueSlider" })
            {
                FrameworkElement child = (FrameworkElement)editor.FindName(name);
                Point origin = child.TranslatePoint(new Point(), editor);
                Assert.True(child.ActualWidth > 40);
                Assert.InRange(origin.X, 0, width);
                Assert.True(origin.X + child.ActualWidth <= width + 1);
            }
        });
    }

    private static void RunSta(Action action)
    {
        Exception? failure = null;
        Thread thread = new(() =>
        {
            try { action(); }
            catch (Exception ex) { failure = ex; }
        });
        thread.SetApartmentState(ApartmentState.STA);
        thread.Start();
        Assert.True(thread.Join(TimeSpan.FromSeconds(10)), "UI test timed out.");
        if (failure is not null) ExceptionDispatchInfo.Capture(failure).Throw();
    }
}
