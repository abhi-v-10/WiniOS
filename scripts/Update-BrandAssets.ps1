#Requires -Version 7.4
# Format/size conversion of the user-supplied artwork; no redraw.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
if (-not $IsWindows) { throw 'Run this asset conversion on Windows.' }
Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent
$source = [Drawing.Bitmap]::new((Join-Path $root 'src\WiniOS_AppIcon.png'))
$output = [IO.MemoryStream]::new()
$writer = [IO.BinaryWriter]::new($output)
$sizes = @(16,24,32,48,64,128,256)
$payloads = @()
try {
    foreach ($size in $sizes) {
        $bitmap = [Drawing.Bitmap]::new($size,$size,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $attributes = [Drawing.Imaging.ImageAttributes]::new()
        try {
            $graphics.Clear([Drawing.Color]::Transparent)
            $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
            $graphics.CompositingQuality = [Drawing.Drawing2D.CompositingQuality]::HighQuality
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $attributes.SetWrapMode([Drawing.Drawing2D.WrapMode]::TileFlipXY)
            $graphics.DrawImage($source,[Drawing.Rectangle]::new(0,0,$size,$size),0,0,$source.Width,$source.Height,[Drawing.GraphicsUnit]::Pixel,$attributes)
            $stream = [IO.MemoryStream]::new()
            $entry = [IO.BinaryWriter]::new($stream)
            try {
                $stride = [int]([math]::Ceiling($size/32.0)*4)
                $entry.Write([uint32]40);$entry.Write([int]$size);$entry.Write([int]($size*2))
                $entry.Write([uint16]1);$entry.Write([uint16]32)
                foreach ($value in @([uint32]0,[uint32]($size*$size*4),[uint32]0,[uint32]0,[uint32]0,[uint32]0)) {$entry.Write($value)}
                for ($y=$size-1;$y-ge 0;$y--) {for ($x=0;$x-lt $size;$x++) {
                    $pixel=$bitmap.GetPixel($x,$y)
                    $entry.Write([byte]$pixel.B);$entry.Write([byte]$pixel.G);$entry.Write([byte]$pixel.R);$entry.Write([byte]$pixel.A)
                }}
                for ($y=$size-1;$y-ge 0;$y--) {
                    $mask=[byte[]]::new($stride)
                    for($x=0;$x-lt $size;$x++){if($bitmap.GetPixel($x,$y).A-eq 0){$mask[[int][math]::Floor($x/8)]=$mask[[int][math]::Floor($x/8)]-bor(0x80-shr($x%8))}}
                    $entry.Write($mask)
                }
                $payloads += ,$stream.ToArray()
                if ($size-eq 32) {
                    $rgba=[byte[]]::new(32*32*4)
                    for($y=0;$y-lt 32;$y++){for($x=0;$x-lt 32;$x++){$pixel=$bitmap.GetPixel($x,$y);$offset=($y*32+$x)*4;$rgba[$offset]=$pixel.R;$rgba[$offset+1]=$pixel.G;$rgba[$offset+2]=$pixel.B;$rgba[$offset+3]=$pixel.A}}
                    [IO.File]::WriteAllBytes((Join-Path $root 'resources\winios-32.rgba'),$rgba)
                }
            } finally {$entry.Dispose();$stream.Dispose()}
        } finally {$attributes.Dispose();$graphics.Dispose();$bitmap.Dispose()}
    }
    $writer.Write([uint16]0);$writer.Write([uint16]1);$writer.Write([uint16]$sizes.Count)
    $offset=6+16*$sizes.Count
    for($i=0;$i-lt $sizes.Count;$i++){
        $writer.Write([byte]($sizes[$i]%256));$writer.Write([byte]($sizes[$i]%256));$writer.Write([uint16]0)
        $writer.Write([uint16]1);$writer.Write([uint16]32);$writer.Write([uint32]$payloads[$i].Length);$writer.Write([uint32]$offset)
        $offset+=$payloads[$i].Length
    }
    foreach($bytes in $payloads){$writer.Write([byte[]]$bytes)}
    [IO.File]::WriteAllBytes((Join-Path $root 'resources\winios.ico'),$output.ToArray())
    Write-Output 'Updated multi-resolution icon and tray fallback from src\WiniOS_AppIcon.png.'
} finally {$source.Dispose();$writer.Dispose();$output.Dispose()}
