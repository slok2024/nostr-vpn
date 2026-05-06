using System.Windows;
using System.Windows.Media;
using NostrVpn.Windows.Core;

namespace NostrVpn.Windows.Controls;

public sealed class QrCodeView : FrameworkElement
{
    public static readonly DependencyProperty MatrixProperty = DependencyProperty.Register(
        nameof(Matrix),
        typeof(QrMatrix),
        typeof(QrCodeView),
        new FrameworkPropertyMetadata(null, FrameworkPropertyMetadataOptions.AffectsRender));

    public QrMatrix? Matrix
    {
        get => (QrMatrix?)GetValue(MatrixProperty);
        set => SetValue(MatrixProperty, value);
    }

    protected override System.Windows.Size MeasureOverride(System.Windows.Size availableSize)
    {
        var side = Math.Min(availableSize.Width, availableSize.Height);
        if (double.IsInfinity(side) || side <= 0)
        {
            side = 160;
        }
        return new System.Windows.Size(side, side);
    }

    protected override void OnRender(DrawingContext drawingContext)
    {
        base.OnRender(drawingContext);
        var side = Math.Min(ActualWidth, ActualHeight);
        var originX = (ActualWidth - side) / 2;
        var originY = (ActualHeight - side) / 2;
        drawingContext.DrawRectangle(Brushes.White, null, new Rect(originX, originY, side, side));

        var matrix = Matrix;
        if (matrix is null || matrix.Width <= 0 || matrix.Cells.Count != matrix.Width * matrix.Width)
        {
            DrawPlaceholder(drawingContext, originX, originY, side);
            return;
        }

        const int quietModules = 4;
        var moduleCount = matrix.Width + quietModules * 2;
        var cell = side / moduleCount;
        var size = Math.Ceiling(cell);
        for (var y = 0; y < matrix.Width; y++)
        {
            for (var x = 0; x < matrix.Width; x++)
            {
                if (!matrix.Cells[y * matrix.Width + x])
                {
                    continue;
                }
                drawingContext.DrawRectangle(
                    Brushes.Black,
                    null,
                    new Rect(originX + (x + quietModules) * cell, originY + (y + quietModules) * cell, size, size));
            }
        }
    }

    private static void DrawPlaceholder(DrawingContext drawingContext, double x, double y, double side)
    {
        var pen = new Pen(new SolidColorBrush(Color.FromRgb(190, 199, 209)), 1);
        drawingContext.DrawRectangle(new SolidColorBrush(Color.FromRgb(248, 250, 252)), pen, new Rect(x, y, side, side));
        var text = new FormattedText(
            "QR",
            System.Globalization.CultureInfo.CurrentCulture,
            FlowDirection.LeftToRight,
            new Typeface("Segoe UI"),
            22,
            new SolidColorBrush(Color.FromRgb(104, 113, 124)),
            VisualTreeHelper.GetDpi(System.Windows.Application.Current.MainWindow).PixelsPerDip);
        drawingContext.DrawText(text, new Point(x + (side - text.Width) / 2, y + (side - text.Height) / 2));
    }
}
