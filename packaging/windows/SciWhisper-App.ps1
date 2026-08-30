$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$binary = Join-Path $scriptDir "sciwhisper.exe"

if (-not (Test-Path $binary)) {
    Add-Type -AssemblyName System.Windows.Forms
    [System.Windows.Forms.MessageBox]::Show(
        "Рядом с launcher не найден sciwhisper.exe.",
        "Si-Witch — ошибка",
        "OK",
        "Error"
    ) | Out-Null
    exit 1
}

Start-Process -FilePath $binary -ArgumentList @("app") -WorkingDirectory $scriptDir -WindowStyle Hidden
