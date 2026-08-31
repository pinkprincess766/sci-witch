Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

[System.Windows.Forms.Application]::EnableVisualStyles()

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$binary = Join-Path $scriptDir "sciwhisper.exe"
if (-not (Test-Path $binary)) {
    $binary = Join-Path $scriptDir "..\..\target\release\sciwhisper.exe"
}
if (-not (Test-Path $binary)) {
    [System.Windows.Forms.MessageBox]::Show(
        "Рядом с приложением не найден sciwhisper.exe.",
        "Si-Witch — ошибка",
        "OK",
        "Error"
    ) | Out-Null
    exit 1
}

function Invoke-SciWhisper {
    param([string[]]$Arguments)
    $result = & $binary @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw ($result -join [Environment]::NewLine)
    }
    return $result
}

function Read-Settings {
    $values = @{}
    foreach ($line in (Invoke-SciWhisper -Arguments @("settings", "show"))) {
        if ($line -match '^\s{2}([a-z_]+):\s+(.*)$') {
            $values[$matches[1]] = $matches[2].Trim()
        }
    }
    return $values
}

$settings = Read-Settings

$form = New-Object System.Windows.Forms.Form
$form.Text = "Si-Witch — настройки"
$form.ClientSize = New-Object System.Drawing.Size(620, 606)
$form.StartPosition = "CenterScreen"
$form.FormBorderStyle = "FixedDialog"
$form.MaximizeBox = $false
$form.BackColor = [System.Drawing.Color]::FromArgb(248, 242, 226)
$form.Font = New-Object System.Drawing.Font("Segoe UI", 10)
$iconPath = Join-Path $scriptDir "SciWhisper.ico"
if (Test-Path $iconPath) {
    $form.Icon = New-Object System.Drawing.Icon($iconPath)
}

$title = New-Object System.Windows.Forms.Label
$title.Text = "Si-Witch"
$title.Font = New-Object System.Drawing.Font("Segoe UI Semibold", 22)
$title.ForeColor = [System.Drawing.Color]::FromArgb(34, 45, 52)
$title.Location = New-Object System.Drawing.Point(28, 20)
$title.AutoSize = $true
$form.Controls.Add($title)

$subtitle = New-Object System.Windows.Forms.Label
$subtitle.Text = "Локальная научная диктовка"
$subtitle.ForeColor = [System.Drawing.Color]::FromArgb(95, 100, 96)
$subtitle.Location = New-Object System.Drawing.Point(31, 61)
$subtitle.AutoSize = $true
$form.Controls.Add($subtitle)

function Add-Label {
    param([string]$Text, [int]$Y)
    $label = New-Object System.Windows.Forms.Label
    $label.Text = $Text
    $label.Location = New-Object System.Drawing.Point(32, $Y)
    $label.Size = New-Object System.Drawing.Size(170, 25)
    $label.ForeColor = [System.Drawing.Color]::FromArgb(47, 52, 50)
    $form.Controls.Add($label)
}

function Add-TextBox {
    param([string]$Value, [int]$Y)
    $box = New-Object System.Windows.Forms.TextBox
    $box.Text = $Value
    $box.Location = New-Object System.Drawing.Point(205, $Y - 3)
    $box.Size = New-Object System.Drawing.Size(375, 27)
    $form.Controls.Add($box)
    return $box
}

function Add-ComboBox {
    param([string[]]$Items, [string]$Value, [int]$Y)
    $combo = New-Object System.Windows.Forms.ComboBox
    $combo.DropDownStyle = "DropDownList"
    [void]$combo.Items.AddRange($Items)
    $combo.SelectedItem = $Value
    if ($combo.SelectedIndex -lt 0) { $combo.SelectedIndex = 0 }
    $combo.Location = New-Object System.Drawing.Point(205, $Y - 3)
    $combo.Size = New-Object System.Drawing.Size(375, 27)
    $form.Controls.Add($combo)
    return $combo
}

Add-Label "Научный домен" 112
$domain = Add-ComboBox @("auto", "chemistry", "mathematics", "physics", "plain") $settings.domain 112
Add-Label "Формат вставки" 153
$output = Add-ComboBox @("auto", "unicode", "latex", "word") $settings.output 153
Add-Label "Язык распознавания" 194
$language = Add-TextBox $settings.language 194
Add-Label "Локальная модель" 235
$modelValue = $settings.model
if ($modelValue -eq "default local model") { $modelValue = "" }
$model = Add-TextBox $modelValue 235
Add-Label "Запись по удержанию" 294
$ptt = Add-TextBox $settings.ptt 294

$doubleControl = New-Object System.Windows.Forms.CheckBox
$doubleControl.Text = "Двойной Control запускает и завершает запись"
$doubleControl.Checked = ($settings.double_control -eq "true")
$doubleControl.Location = New-Object System.Drawing.Point(205, 329)
$doubleControl.Size = New-Object System.Drawing.Size(375, 28)
$form.Controls.Add($doubleControl)

Add-Label "Быстрый LaTeX" 376
$pttLatex = Add-TextBox $settings.ptt_latex 376
Add-Label "Быстрый Word" 417
$pttWord = Add-TextBox $settings.ptt_word 417

$history = New-Object System.Windows.Forms.CheckBox
$history.Text = "Хранить локальную историю распознаваний"
$history.Checked = ($settings.persist_history -eq "true")
$history.Location = New-Object System.Drawing.Point(205, 458)
$history.Size = New-Object System.Drawing.Size(375, 28)
$form.Controls.Add($history)

$status = New-Object System.Windows.Forms.Label
$status.Text = "Все настройки хранятся локально."
$status.ForeColor = [System.Drawing.Color]::FromArgb(95, 100, 96)
$status.Location = New-Object System.Drawing.Point(32, 499)
$status.Size = New-Object System.Drawing.Size(360, 25)
$form.Controls.Add($status)

$doctor = New-Object System.Windows.Forms.Button
$doctor.Text = "Диагностика"
$doctor.Location = New-Object System.Drawing.Point(32, 540)
$doctor.Size = New-Object System.Drawing.Size(130, 36)
$doctor.Add_Click({
    try {
        $report = (Invoke-SciWhisper -Arguments @("doctor")) -join [Environment]::NewLine
        [System.Windows.Forms.MessageBox]::Show($report, "Si-Witch — диагностика", "OK", "Information") | Out-Null
    } catch {
        [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, "Ошибка диагностики", "OK", "Error") | Out-Null
    }
})
$form.Controls.Add($doctor)

$cancel = New-Object System.Windows.Forms.Button
$cancel.Text = "Отмена"
$cancel.DialogResult = [System.Windows.Forms.DialogResult]::Cancel
$cancel.Location = New-Object System.Drawing.Point(354, 540)
$cancel.Size = New-Object System.Drawing.Size(105, 36)
$form.Controls.Add($cancel)
$form.CancelButton = $cancel

$save = New-Object System.Windows.Forms.Button
$save.Text = "Сохранить"
$save.Location = New-Object System.Drawing.Point(475, 540)
$save.Size = New-Object System.Drawing.Size(105, 36)
$save.BackColor = [System.Drawing.Color]::FromArgb(206, 143, 55)
$save.FlatStyle = "Flat"
$save.Add_Click({
    try {
        Invoke-SciWhisper -Arguments @("settings", "set", "domain", $domain.SelectedItem) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "output", $output.SelectedItem) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "language", $language.Text) | Out-Null
        $modelSetting = $model.Text.Trim()
        if ([string]::IsNullOrWhiteSpace($modelSetting)) { $modelSetting = "default" }
        Invoke-SciWhisper -Arguments @("settings", "set", "model", $modelSetting) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "ptt", $ptt.Text) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "double_control", $doubleControl.Checked.ToString().ToLowerInvariant()) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "ptt_latex", $pttLatex.Text) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "ptt_word", $pttWord.Text) | Out-Null
        Invoke-SciWhisper -Arguments @("settings", "set", "persist_history", $history.Checked.ToString().ToLowerInvariant()) | Out-Null
        $status.Text = "Сохранено. Перезапустите приложение."
        $status.ForeColor = [System.Drawing.Color]::FromArgb(49, 111, 74)
    } catch {
        [System.Windows.Forms.MessageBox]::Show($_.Exception.Message, "Настройки не сохранены", "OK", "Error") | Out-Null
    }
})
$form.Controls.Add($save)
$form.AcceptButton = $save

[void]$form.ShowDialog()
