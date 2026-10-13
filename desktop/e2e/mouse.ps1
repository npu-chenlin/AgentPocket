param([int]$FromX, [int]$FromY, [int]$ToX, [int]$ToY, [switch]$Drag)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class AgentPocketTestMouse {
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint flags, uint dx, uint dy, uint data, UIntPtr extra);
}
'@
[AgentPocketTestMouse]::SetCursorPos($FromX, $FromY) | Out-Null
Start-Sleep -Milliseconds 120
[AgentPocketTestMouse]::mouse_event(2, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 200
if ($Drag) {
  for ($i = 1; $i -le 30; $i++) {
    [AgentPocketTestMouse]::SetCursorPos([int]($FromX + ($ToX - $FromX) * $i / 30), [int]($FromY + ($ToY - $FromY) * $i / 30)) | Out-Null
    Start-Sleep -Milliseconds 25
  }
}
Start-Sleep -Milliseconds 150
[AgentPocketTestMouse]::mouse_event(4, 0, 0, 0, [UIntPtr]::Zero)
