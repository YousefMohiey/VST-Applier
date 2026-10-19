# Deploy the GPU denoise VST2 plugin next to the app's other plugins.
#
#   powershell -ExecutionPolicy Bypass -File installer\deploy-gpu-plugin.ps1
#   powershell ... -OrtDir "C:\path\to\onnxruntime\capi"      # other machine
#
# README: crates/applier-denoise-gpu-vst2/README.md
#
# The plugin loads onnxruntime.dll from its own folder (absolute path) and
# pre-loads DirectML.dll from the same folder before the first DirectML
# session, so all four files must sit side by side.

param(
    [string]$OrtDir = "$env:LOCALAPPDATA\Programs\Python\Python312\Lib\site-packages\onnxruntime\capi",
    [string]$Destination = "$PSScriptRoot\app\Plugins\VST-Applier"
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

Write-Host "== build =="
Push-Location $root
try {
    cargo build -p applier-denoise-gpu-vst2 --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
}

$dll = Join-Path $root "target\release\VSTApplierDenoiseGpu.dll"
if (-not (Test-Path $dll)) { throw "missing $dll" }

$ortFiles = @("onnxruntime.dll", "DirectML.dll", "onnxruntime_providers_shared.dll")
foreach ($f in $ortFiles) {
    $src = Join-Path $OrtDir $f
    if (-not (Test-Path $src)) {
        throw "missing $src - pass -OrtDir with the folder that holds onnxruntime.dll (pip package onnxruntime-directml 1.24.4)"
    }
}

if (-not (Test-Path $Destination)) {
    New-Item -ItemType Directory -Path $Destination | Out-Null
}

Write-Host "== deploy to $Destination =="
Copy-Item $dll (Join-Path $Destination "Yousef Noise Suppression GPU.dll") -Force
foreach ($f in $ortFiles) {
    Copy-Item (Join-Path $OrtDir $f) (Join-Path $Destination $f) -Force
}

Get-ChildItem $Destination |
    Where-Object { $_.Name -like "*Noise Suppression GPU*" -or $_.Name -like "onnx*" -or $_.Name -eq "DirectML.dll" } |
    ForEach-Object { Write-Host ("  {0,-45} {1,12:N0} bytes" -f $_.Name, $_.Length) }

Write-Host "done."
