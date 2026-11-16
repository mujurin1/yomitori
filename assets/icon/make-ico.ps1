# yomitori.ico を作る（開発の時だけ手で流す。ビルドでは走らない）。pwsh 7 で: pwsh -File assets/icon/make-ico.ps1
# icon-16.txt / icon-20.txt / icon-24.txt（ドット絵。1 文字 1 画素）を直したら、これを流し直せば yomitori.ico ができ直す
#   B = #0078D4、W = 白、L = #80BCE9、. = 透明
# マス目のエディタで描いた絵のスクリーンショットからは from-image.ps1 で txt を作れる（B・W・. だけを読む）:
#   pwsh -File assets/icon/from-image.ps1 -img <画像> -out assets/icon/icon-24.txt -n 24
# 16・20・24 はドット絵をそのまま、32・40・48・64・256 は下の Draw で描く（青い角丸の四角・白い四隅のカギ・白い「あ」）
Add-Type -AssemblyName System.Drawing
$here = $PSScriptRoot

function FromText([string]$path) {
  $rows = @(Get-Content -LiteralPath $path | Where-Object { $_.Length -gt 0 })
  $s = $rows.Count
  $colors = @{
    'B' = [System.Drawing.Color]::FromArgb(255, 0x00, 0x78, 0xD4)
    'W' = [System.Drawing.Color]::FromArgb(255, 0xFF, 0xFF, 0xFF)
    'L' = [System.Drawing.Color]::FromArgb(255, 0x80, 0xBC, 0xE9)
    '.' = [System.Drawing.Color]::FromArgb(0, 0, 0, 0)
  }
  $bmp = [System.Drawing.Bitmap]::new($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  for ($y = 0; $y -lt $s; $y++) {
    if ($rows[$y].Length -ne $s) { throw "$path の $($y + 1) 行目が $s 文字ではありません" }
    for ($x = 0; $x -lt $s; $x++) {
      $c = [string]$rows[$y][$x]
      if (-not $colors.ContainsKey($c)) { throw "$path の $($y + 1) 行目に知らない文字 '$c'" }
      $bmp.SetPixel($x, $y, $colors[$c])
    }
  }
  $bmp
}

# p: r 角丸の半径、m カギの余白、L カギの腕の長さ、w 線の太さ、jr カギの角の丸み、fs 文字の大きさ（どれも一辺に対する比）
function Draw([int]$s, $p) {
  $bmp = [System.Drawing.Bitmap]::new($s, $s, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.TextRenderingHint = 'AntiAlias'; $g.PixelOffsetMode = 'HighQuality'
  $blue = [System.Drawing.Color]::FromArgb(255, 0, 120, 212); $white = [System.Drawing.Color]::White
  $r = $s * $p.r; $d = $r * 2; $e = $s - 0.5
  $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
  $path.AddArc(0.5, 0.5, $d, $d, 180, 90); $path.AddArc($e - $d, 0.5, $d, $d, 270, 90); $path.AddArc($e - $d, $e - $d, $d, $d, 0, 90); $path.AddArc(0.5, $e - $d, $d, $d, 90, 90); $path.CloseFigure()
  $g.FillPath([System.Drawing.SolidBrush]::new($blue), $path)
  $w = [Math]::Max(1.2, $s * $p.w); $pen = [System.Drawing.Pen]::new($white, $w); $pen.StartCap = 'Round'; $pen.EndCap = 'Round'; $pen.LineJoin = 'Round'
  $m = $s * $p.m + $w / 2; $L = $s * $p.L; $jr = $s * $p.jr; $a = $m; $b = $s - $m
  foreach ($c in @(@($a,$a,1,1),@($b,$a,-1,1),@($a,$b,1,-1),@($b,$b,-1,-1))) {
    $x=$c[0]; $y=$c[1]; $dx=$c[2]; $dy=$c[3]; $bp = [System.Drawing.Drawing2D.GraphicsPath]::new()
    if ($jr -gt 0.5) {
      $bp.AddLine([float]$x, [float]($y + $dy*$L), [float]$x, [float]($y + $dy*$jr))
      $rx = if ($dx -gt 0) { $x } else { $x - 2*$jr }; $ry = if ($dy -gt 0) { $y } else { $y - 2*$jr }
      $start = if ($dx -gt 0) { 180 } else { 0 }; $sweep = if (($dx*$dy) -gt 0) { 90 } else { -90 }
      $bp.AddArc([float]$rx,[float]$ry,[float](2*$jr),[float](2*$jr),[float]$start,[float]$sweep)
      $bp.AddLine([float]($x + $dx*$jr), [float]$y, [float]($x + $dx*$L), [float]$y)
    } else { $bp.AddLines([System.Drawing.PointF[]]@([System.Drawing.PointF]::new($x, $y + $dy*$L), [System.Drawing.PointF]::new($x, $y), [System.Drawing.PointF]::new($x + $dx*$L, $y))) }
    $g.DrawPath($pen, $bp) }
  $font = [System.Drawing.Font]::new('Yu Gothic UI', [float]($s * $p.fs), [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
  $sf = [System.Drawing.StringFormat]::new(); $sf.Alignment = 'Center'; $sf.LineAlignment = 'Center'
  $g.DrawString('あ', $font, [System.Drawing.SolidBrush]::new($white), [System.Drawing.RectangleF]::new(0, [float]($s*0.03), $s, $s), $sf)
  $g.Dispose(); $bmp }

function Params([int]$s) {
  return @{ r=0.22; m=0.14; L=0.18; w=0.09; jr=0.07; fs=0.48 }
}

function PngBytes($bmp) {
  $ms = [System.IO.MemoryStream]::new()
  $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $ms.ToArray()
}

$images = [System.Collections.Generic.List[object]]::new()
foreach ($s in 16, 20, 24, 32, 40, 48, 64, 256) {
  $bmp = switch ($s) {
    16 { FromText (Join-Path $here 'icon-16.txt') }
    20 { FromText (Join-Path $here 'icon-20.txt') }
    24 { FromText (Join-Path $here 'icon-24.txt') }
    default { Draw $s (Params $s) }
  }
  if ($bmp.Width -ne $s) { throw "icon-$s の大きさが $($bmp.Width) です" }
  $images.Add([pscustomobject]@{ Size = $s; Bytes = (PngBytes $bmp) })
  $bmp.Dispose()
}

# .ico: ICONDIR（6 バイト）+ ICONDIRENTRY（16 バイト × 枚数）+ 画像（どれも PNG。Vista 以降はどの大きさも PNG を読める）
$out = [System.IO.MemoryStream]::new()
$w = [System.IO.BinaryWriter]::new($out)
$w.Write([uint16]0); $w.Write([uint16]1); $w.Write([uint16]$images.Count)
$offset = 6 + 16 * $images.Count
foreach ($img in $images) {
  $s = $img.Size; $len = $img.Bytes.Length
  $dim = if ($s -ge 256) { 0 } else { $s }
  $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
  $w.Write([uint16]1); $w.Write([uint16]32); $w.Write([uint32]$len); $w.Write([uint32]$offset)
  $offset += $len
}
foreach ($img in $images) { $w.Write([byte[]]$img.Bytes) }
$w.Flush()
[System.IO.File]::WriteAllBytes((Join-Path $here 'yomitori.ico'), $out.ToArray())
"yomitori.ico: $(($images | ForEach-Object Size) -join ', ') px, $($out.Length) bytes"
