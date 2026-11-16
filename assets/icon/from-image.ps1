param([string]$img, [string]$out, [int]$n = 24)
Add-Type -AssemblyName System.Drawing
$b = [System.Drawing.Bitmap]::FromFile($img); $Wd = $b.Width; $Ht = $b.Height
function IsLine($c) { [Math]::Abs($c.R - $c.G) -lt 25 -and [Math]::Abs($c.G - $c.B) -lt 25 -and $c.R -lt 140 }
function Centers([int]$len, [scriptblock]$get, [int]$other) {
  $isl = foreach ($i in 0..($len-1)) { $cnt = 0; foreach ($j in 0..($other-1)) { if (IsLine (& $get $i $j)) { $cnt++ } }; $cnt -gt $other * 0.5 }
  $cs = @(); $start = -1
  for ($i = 0; $i -lt $len; $i++) { if (-not $isl[$i]) { if ($start -lt 0) { $start = $i } } elseif ($start -ge 0) { $cs += [int](($start + $i - 1) / 2); $start = -1 } }
  if ($start -ge 0) { $cs += [int](($start + $len - 1) / 2) }
  , $cs }
$xs = Centers $Wd { param($i,$j) $b.GetPixel($i,$j) } $Ht
$ys = Centers $Ht { param($i,$j) $b.GetPixel($j,$i) } $Wd
"cells found: $($xs.Count) x $($ys.Count)" | Write-Host
if ($xs.Count -ne $n -or $ys.Count -ne $n) { throw "expected $n x $n cells" }
$g = New-Object 'char[,]' $n, $n
for ($y=0;$y -lt $n;$y++){ for ($x=0;$x -lt $n;$x++){ $c = $b.GetPixel($xs[$x], $ys[$y])
  $g[$y,$x] = if ($c.B -gt 150 -and $c.R -lt 100) { 'B' } elseif ($c.R -gt 200 -and $c.G -gt 200 -and $c.B -gt 200) { 'W' } else { '?' } } }
$b.Dispose()
$q = [System.Collections.Generic.Queue[int[]]]::new()
foreach ($i in 0..($n-1)) { foreach ($p in [int[]]@(0,$i), [int[]]@(($n-1),$i), [int[]]@($i,0), [int[]]@($i,($n-1))) { if ($g[$p[0],$p[1]] -eq 'W') { $g[$p[0],$p[1]] = '.'; $q.Enqueue($p) } } }
while ($q.Count) { $p = $q.Dequeue(); foreach ($d in [int[]]@(1,0), [int[]]@(-1,0), [int[]]@(0,1), [int[]]@(0,-1)) { $yy = $p[0]+$d[0]; $xx = $p[1]+$d[1]
  if ($yy -ge 0 -and $yy -lt $n -and $xx -ge 0 -and $xx -lt $n -and $g[$yy,$xx] -eq 'W') { $g[$yy,$xx] = '.'; $q.Enqueue([int[]]@($yy,$xx)) } } }
$lines = for ($y=0;$y -lt $n;$y++){ -join (0..($n-1) | % { $g[$y,$_] }) }
$lines | Set-Content $out; $lines
