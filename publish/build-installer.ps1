$exitCode = 0

function Resolve-IsccPath {
    $knownPaths = @(
        "ISCC.exe",
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\JRSoftware\Inno Setup 6\ISCC.exe"
    )

    foreach ($path in $knownPaths) {
        if ([string]::IsNullOrWhiteSpace($path)) {
            continue
        }

        $command = Get-Command $path -ErrorAction SilentlyContinue
        if ($command) {
            return $command.Source
        }

        if (Test-Path -LiteralPath $path) {
            return $path
        }
    }

    $registryPaths = @(
        "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1",
        "HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1",
        "HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1",
        "HKCU:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Inno Setup 6_is1"
    )

    foreach ($registryPath in $registryPaths) {
        $installLocation = (Get-ItemProperty -Path $registryPath -ErrorAction SilentlyContinue).InstallLocation
        if ([string]::IsNullOrWhiteSpace($installLocation)) {
            continue
        }

        $candidate = Join-Path $installLocation "ISCC.exe"
        if (Test-Path -LiteralPath $candidate) {
            return $candidate
        }
    }

    return $null
}

try {
    $ErrorActionPreference = "Stop"

    $scriptDir = $PSScriptRoot
    $issPath = Join-Path $scriptDir "VstApplier.iss"
    $publishDir = Join-Path $scriptDir "app"

    if (-not (Test-Path -LiteralPath $publishDir)) {
        throw "Publish folder not found: $publishDir. Run publish\publish-self-contained.ps1 first."
    }

    $iscc = Resolve-IsccPath

    if (-not $iscc) {
        throw "ISCC.exe was not found. Install Inno Setup 6 or add ISCC.exe to PATH."
    }

    Write-Host "Building installer with Inno Setup..." -ForegroundColor Cyan
    Write-Host "Compiler: $iscc"
    Write-Host "Script:   $issPath"

    Push-Location $scriptDir
    try {
        & $iscc "VstApplier.iss"
        if ($LASTEXITCODE -ne 0) {
            throw "ISCC.exe exited with code $LASTEXITCODE."
        }
    }
    finally {
        Pop-Location
    }

    Write-Host ""
    Write-Host "Installer completed successfully." -ForegroundColor Green
    Write-Host "Output: $(Join-Path $scriptDir 'VstApplier_v1.4.exe')"
}
catch {
    $exitCode = 1
    Write-Host ""
    Write-Host "Installer build failed:" -ForegroundColor Red
    Write-Host $_.Exception.Message -ForegroundColor Red
}
finally {
    Write-Host ""
    Read-Host "Press Enter to close"
}

if ($exitCode -ne 0) {
    exit $exitCode
}
