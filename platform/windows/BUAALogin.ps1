# Native Windows Forms UI, DPAPI credential storage and notification-area icon.
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$script:worker = $null
$script:mode = ''
$script:stopping = $false
$script:closing = $false
$settingsPath = Join-Path $env:LOCALAPPDATA 'BUAALogin\settings.json'
$form = New-Object System.Windows.Forms.Form
$form.Text = '北航校园网 · 自动重连'
$form.Size = New-Object System.Drawing.Size(640, 580)
$form.MinimumSize = $form.Size
$form.StartPosition = 'CenterScreen'
$form.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 10)
function Add-Label($text, $x, $y, $width = 180) {
    $control = New-Object System.Windows.Forms.Label
    $control.Text = $text
    $control.Location = New-Object System.Drawing.Point($x, $y)
    $control.Size = New-Object System.Drawing.Size($width, 28)
    $form.Controls.Add($control)
    return $control
}
function Add-Input($y, $value = '') {
    $control = New-Object System.Windows.Forms.TextBox
    $control.Location = New-Object System.Drawing.Point(205, $y)
    $control.Size = New-Object System.Drawing.Size(390, 28)
    $control.Text = $value
    $form.Controls.Add($control)
    return $control
}
$title = Add-Label '北航校园网' 24 18 500
$title.Font = New-Object System.Drawing.Font('Microsoft YaHei UI', 18, [System.Drawing.FontStyle]::Bold)
$null = Add-Label '定期检查，掉线自动登录' 24 57 550
$null = Add-Label '账号' 24 100
$account = Add-Input 97
$null = Add-Label '密码' 24 142
$password = Add-Input 139
$password.UseSystemPasswordChar = $true
$null = Add-Label '检查间隔（秒）' 24 184
$interval = Add-Input 181 '300'
$null = Add-Label '校园网接口' 24 226
$interface = New-Object System.Windows.Forms.ComboBox
$interface.Location = New-Object System.Drawing.Point(205, 223)
$interface.Size = New-Object System.Drawing.Size(278, 28)
$interface.DropDownStyle = 'DropDownList'
$interface.DisplayMember = 'Label'
$null = $interface.Items.Add([pscustomobject]@{Label='自动选择（依次检测接口）'; Binding=$null})
$interface.SelectedIndex = 0
$form.Controls.Add($interface)
$detect = New-Object System.Windows.Forms.Button
$detect.Text = '检测校园网'
$detect.Location = New-Object System.Drawing.Point(492, 221)
$detect.Size = New-Object System.Drawing.Size(104, 31)
$form.Controls.Add($detect)
$null = Add-Label '网关 IPv4' 24 268
$gateway = Add-Input 265 '10.200.21.4'
$remember = New-Object System.Windows.Forms.CheckBox
$remember.Text = '记住账号密码（当前 Windows 用户加密保护）'
$remember.Location = New-Object System.Drawing.Point(24, 307)
$remember.Size = New-Object System.Drawing.Size(570, 28)
$form.Controls.Add($remember)
$start = New-Object System.Windows.Forms.Button
$start.Text = '开始自动重连'
$start.Location = New-Object System.Drawing.Point(24, 350)
$start.Size = New-Object System.Drawing.Size(160, 36)
$form.Controls.Add($start)
$stop = New-Object System.Windows.Forms.Button
$stop.Text = '停止'
$stop.Location = New-Object System.Drawing.Point(199, 350)
$stop.Size = New-Object System.Drawing.Size(90, 36)
$stop.Enabled = $false
$form.Controls.Add($stop)
$hide = New-Object System.Windows.Forms.Button
$hide.Text = '最小化至托盘'
$hide.Location = New-Object System.Drawing.Point(304, 350)
$hide.Size = New-Object System.Drawing.Size(155, 36)
$form.Controls.Add($hide)
$state = Add-Label '未启动' 24 399 570
$history = New-Object System.Windows.Forms.TextBox
$history.Location = New-Object System.Drawing.Point(24, 435)
$history.Size = New-Object System.Drawing.Size(570, 90)
$history.Multiline = $true
$history.ReadOnly = $true
$history.ScrollBars = 'Vertical'
$history.Anchor = 'Top, Bottom, Left, Right'
$form.Controls.Add($history)
$inputs = @($account, $password, $interval, $interface, $gateway, $remember)
$tray = New-Object System.Windows.Forms.NotifyIcon
$tray.Icon = [System.Drawing.SystemIcons]::Information
$tray.Text = '北航校园网 · 未启动'
$tray.Visible = $true
$menu = New-Object System.Windows.Forms.ContextMenuStrip
$trayStatus = $menu.Items.Add('未启动')
$trayStatus.Enabled = $false
$null = $menu.Items.Add('-')
$showMenu = $menu.Items.Add('显示窗口')
$exitMenu = $menu.Items.Add('退出')
$tray.ContextMenuStrip = $menu
function Show-Window { $form.Show(); $form.WindowState = 'Normal'; $form.Activate() }
function Hide-Window { $form.Hide() }
$tray.Add_DoubleClick({ Show-Window })
$showMenu.Add_Click({ Show-Window })
$hide.Add_Click({ Hide-Window })
$form.Add_Resize({ if ($form.WindowState -eq 'Minimized') { Hide-Window } })

# No GUI polling timer: a blocking pipe reader dispatches native UI events.
Add-Type -ReferencedAssemblies System.Windows.Forms -TypeDefinition @'
using System;
using System.Diagnostics;
using System.Threading;
using System.Windows.Forms;
public static class CampusWorkerReader {
    public static void Watch(Control ui, Process process, Action<string> receive, Action finished) {
        ThreadPool.QueueUserWorkItem(delegate(object unused) {
            try {
                string line;
                while ((line = process.StandardOutput.ReadLine()) != null) {
                    if (!ui.IsDisposed) ui.BeginInvoke(receive, new object[] { line });
                }
                process.WaitForExit();
            } catch { }
            finally { if (!ui.IsDisposed) ui.BeginInvoke(finished); }
        });
    }
}
'@
function Set-Status($message) {
    $state.Text = $message
    $trayStatus.Text = $message
    $tray.Text = ('北航校园网 · ' + $message).Substring(0, [Math]::Min(63, ('北航校园网 · ' + $message).Length))
}
function Record-Status($message) {
    Set-Status $message
    $history.AppendText((Get-Date -Format 'HH:mm:ss') + '  ' + $message + "`r`n")
    if ($history.Lines.Count -gt 120) { $history.Lines = $history.Lines | Select-Object -Last 120 }
}
function Apply-Event($line) {
    try {
        $event = $line | ConvertFrom-Json
        if ($event.state -eq 'interfaces') {
            $old = $interface.SelectedItem.Binding
            $interface.Items.Clear()
            $null = $interface.Items.Add([pscustomobject]@{Label='自动选择（依次检测接口）'; Binding=$null})
            $interface.SelectedIndex = 0
            foreach ($record in $event.records) {
                $index = $interface.Items.Add([pscustomobject]@{Label=($record.label + ' · ' + $record.message); Binding=$record.binding})
                if ($old -and $record.binding -eq $old) { $interface.SelectedIndex = $index }
                Record-Status ($record.label + ' · ' + $record.message)
            }
            Record-Status '接口检测完成'
            return
        }
        $message = $event.message
        if ($event.delay) { $message += ' · ' + $event.delay + ' 秒后检查' }
        if ($event.state -eq 'checking') { Set-Status $message }
        else { Record-Status $message }
    } catch { Set-Status '状态信息无法读取' }
}
function Forget-Credentials {
    if (Test-Path $settingsPath) { Remove-Item -LiteralPath $settingsPath -Force -ErrorAction Stop }
}
function Save-Credentials {
    if (-not $remember.Checked) { Forget-Credentials; return }
    $secure = ConvertTo-SecureString $password.Text -AsPlainText -Force
    # Without -Key, ConvertFrom-SecureString uses Windows DPAPI for the current user.
    $encrypted = ConvertFrom-SecureString $secure
    $saved = @{username=$account.Text.Trim(); password=$encrypted; interval=$interval.Text; gateway=$gateway.Text}
    $directory = Split-Path -Parent $settingsPath
    $null = New-Item -ItemType Directory -Path $directory -Force
    $saved | ConvertTo-Json | Set-Content -LiteralPath ($settingsPath + '.tmp') -Encoding UTF8
    Move-Item -LiteralPath ($settingsPath + '.tmp') -Destination $settingsPath -Force
    $secure.Dispose()
}
function Load-Credentials {
    if (-not (Test-Path $settingsPath)) { return }
    $pointer = [IntPtr]::Zero
    $secure = $null
    try {
        $saved = Get-Content -LiteralPath $settingsPath -Raw -Encoding UTF8 | ConvertFrom-Json
        $secure = ConvertTo-SecureString $saved.password -ErrorAction Stop
        $pointer = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secure)
        $account.Text = $saved.username
        $password.Text = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($pointer)
        if ($saved.interval) { $interval.Text = $saved.interval }
        if ($saved.gateway) { $gateway.Text = $saved.gateway }
        $remember.Checked = $true
    } catch { Record-Status '保存的凭据无法解密，请重新输入' }
    finally {
        if ($pointer -ne [IntPtr]::Zero) { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($pointer) }
        if ($secure) { $secure.Dispose() }
    }
}
function Stop-Worker {
    if ($script:worker -and -not $script:worker.HasExited -and -not $script:stopping) {
        $script:stopping = $true
        $stop.Enabled = $false
        Set-Status '正在停止，等待当前请求结束…'
        try { $script:worker.StandardInput.WriteLine('stop'); $script:worker.StandardInput.Close() } catch { }
    }
}
function Reset-Worker {
    if ($script:worker) { $script:worker.Dispose() }
    $script:worker = $null
    $script:stopping = $false
    foreach ($entry in $inputs) { $entry.Enabled = $true }
    $start.Enabled = $true
    $detect.Enabled = $true
    $stop.Enabled = $false
    if ($script:mode -eq 'monitor' -and $remember.Checked) { Load-Credentials }
    $script:mode = ''
    if ($script:closing) { $form.Close() }
}
function Start-Worker($configuration, $mode) {
    if ($script:worker) { return }
    try {
        $info = New-Object System.Diagnostics.ProcessStartInfo
        $info.FileName = Join-Path $PSScriptRoot 'runtime\python.exe'
        $info.Arguments = '-u "' + (Join-Path $PSScriptRoot 'desktop_worker.py') + '"'
        $info.WorkingDirectory = $PSScriptRoot
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $true
        $info.RedirectStandardInput = $true
        $info.RedirectStandardOutput = $true
        $info.StandardOutputEncoding = New-Object System.Text.UTF8Encoding($false)
        $info.EnvironmentVariables['PYTHONIOENCODING'] = 'utf-8'
        $info.EnvironmentVariables['PYTHONUTF8'] = '1'
        $script:worker = New-Object System.Diagnostics.Process
        $script:worker.StartInfo = $info
        $null = $script:worker.Start()
        $script:mode = $mode
        $json = $configuration | ConvertTo-Json -Compress
        $configuration.Clear()
        # ASCII JSON avoids PowerShell 5.1 stdin encoding ambiguity.
        $ascii = New-Object System.Text.StringBuilder
        foreach ($character in $json.ToCharArray()) {
            if ([int]$character -gt 127) { $null = $ascii.Append(('\u{0:x4}' -f [int]$character)) }
            else { $null = $ascii.Append($character) }
        }
        $script:worker.StandardInput.WriteLine($ascii.ToString())
        $json = $ascii = $null
        if ($mode -eq 'monitor') { $password.Clear() }
        foreach ($entry in $inputs) { $entry.Enabled = $false }
        $start.Enabled = $false
        $detect.Enabled = $false
        $stop.Enabled = $mode -eq 'monitor'
        Set-Status '正在依次检测校园网接口…'
        [CampusWorkerReader]::Watch($form, $script:worker, [Action[string]]{ param($line); Apply-Event $line }, [Action]{
            if ($script:worker -and $script:worker.HasExited -and $script:worker.ExitCode -ne 0) {
                Set-Status '程序退出，请检查配置后重新启动'
            }
            Reset-Worker
        })
    } catch {
        try { if ($script:worker -and -not $script:worker.HasExited) { $script:worker.Kill() } } catch { }
        Reset-Worker
        [System.Windows.Forms.MessageBox]::Show('无法启动，请完整解压发行包后重试。', '启动失败')
    }
}
function Detect-Interfaces {
    Start-Worker @{action='interfaces'; gateway_ip=$gateway.Text.Trim()} 'scan'
}
$start.Add_Click({
    $seconds = 0
    if (-not $account.Text.Trim() -or -not $password.Text -or
        -not [int]::TryParse($interval.Text, [ref]$seconds) -or $seconds -lt 30 -or $seconds -gt 86400) {
        [System.Windows.Forms.MessageBox]::Show('请输入账号密码，检查间隔须在 30–86400 秒之间。', '检查输入')
        return
    }
    $ip = $null
    if (-not [System.Net.IPAddress]::TryParse($gateway.Text.Trim(), [ref]$ip) -or
        $ip.AddressFamily -ne [System.Net.Sockets.AddressFamily]::InterNetwork) {
        [System.Windows.Forms.MessageBox]::Show('请输入有效的网关 IPv4 地址。', '检查输入')
        return
    }
    try { Save-Credentials }
    catch { [System.Windows.Forms.MessageBox]::Show('无法保存加密凭据，请检查用户目录权限。', '保存失败'); return }
    Start-Worker @{username=$account.Text.Trim(); password=$password.Text; interval=$seconds;
                   interface=$interface.SelectedItem.Binding; gateway_ip=$gateway.Text.Trim()} 'monitor'
})
$remember.Add_CheckedChanged({
    if (-not $remember.Checked) {
        try { Forget-Credentials }
        catch { [System.Windows.Forms.MessageBox]::Show('无法删除已保存的凭据。', '删除失败') }
    }
})
$detect.Add_Click({ Detect-Interfaces })
$stop.Add_Click({ Stop-Worker })
$exitMenu.Add_Click({
    $script:closing = $true
    if ($script:worker) { Show-Window; Stop-Worker }
    else { $form.Close() }
})
$form.Add_FormClosing({
    param($sender, $eventArgs)
    if (-not $script:closing) { $eventArgs.Cancel = $true; Hide-Window; return }
    if ($script:worker) { $eventArgs.Cancel = $true; Stop-Worker }
})
$form.Add_Shown({ Load-Credentials; Detect-Interfaces })
$null = $form.ShowDialog()
$tray.Visible = $false
$tray.Dispose()
$menu.Dispose()
$form.Dispose()
