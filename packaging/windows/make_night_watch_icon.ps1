# Builds packaging/windows/night_watch.ico: a light blue octagon with gold rings (the Kyttu's Lab octagon shape, recoloured).
# Run on Windows: powershell -NoProfile -ExecutionPolicy Bypass -File packaging/windows/make_night_watch_icon.ps1
Add-Type -AssemblyName System.Drawing

$out = Join-Path $PSScriptRoot 'night_watch.ico'
$sizes = @(16, 32, 48, 256)
$blue = [System.Drawing.Color]::FromArgb(255, 125, 211, 252)
$gold = [System.Drawing.Color]::FromArgb(255, 250, 204, 21)
$goldDark = [System.Drawing.Color]::FromArgb(255, 234, 179, 8)
$navy = [System.Drawing.Color]::FromArgb(255, 15, 23, 42)

function New-Icon([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)
    $k = $size / 64.0
    $pts = @(
        (New-Object System.Drawing.PointF (19 * $k), (3 * $k)),
        (New-Object System.Drawing.PointF (45 * $k), (3 * $k)),
        (New-Object System.Drawing.PointF (61 * $k), (19 * $k)),
        (New-Object System.Drawing.PointF (61 * $k), (45 * $k)),
        (New-Object System.Drawing.PointF (45 * $k), (61 * $k)),
        (New-Object System.Drawing.PointF (19 * $k), (61 * $k)),
        (New-Object System.Drawing.PointF (3 * $k), (45 * $k)),
        (New-Object System.Drawing.PointF (3 * $k), (19 * $k))
    )
    $fill = New-Object System.Drawing.SolidBrush $blue
    $g.FillPolygon($fill, [System.Drawing.PointF[]]$pts)
    $edge = New-Object System.Drawing.Pen $gold, ([single](4 * $k))
    $edge.LineJoin = [System.Drawing.Drawing2D.LineJoin]::Round
    $g.DrawPolygon($edge, [System.Drawing.PointF[]]$pts)
    $ringPen = New-Object System.Drawing.Pen $goldDark, ([single](5 * $k))
    $r = 18 * $k
    $g.DrawEllipse($ringPen, [single](32 * $k - $r), [single](32 * $k - $r), [single](2 * $r), [single](2 * $r))
    $font = New-Object System.Drawing.Font 'Segoe UI', ([single](30 * $k)), ([System.Drawing.FontStyle]::Bold), ([System.Drawing.GraphicsUnit]::Pixel)
    $ink = New-Object System.Drawing.SolidBrush $navy
    $fmt = New-Object System.Drawing.StringFormat
    $fmt.Alignment = [System.Drawing.StringAlignment]::Center
    $fmt.LineAlignment = [System.Drawing.StringAlignment]::Center
    $g.DrawString('N', $font, $ink, (New-Object System.Drawing.RectangleF 0, 0, $size, $size), $fmt)
    $g.Dispose()
    return $bmp
}

$pngs = @()
foreach ($s in $sizes) {
    $bmp = New-Icon $s
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $pngs += , ($ms.ToArray())
    $bmp.Dispose()
}

$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $ms
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $dim = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$pngs[$i].Length); $bw.Write([uint32]$offset)
    $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $bw.Write($p) }
$bw.Flush()
[System.IO.File]::WriteAllBytes($out, $ms.ToArray())
Write-Output "wrote $out ($((Get-Item $out).Length) bytes, sizes $($sizes -join ', '))"
