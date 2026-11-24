# Indique si le processus donné a ouvert une boîte de dialogue native.
param([int]$ProcessId)
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class Dialogs {
  delegate bool Visit(IntPtr window, IntPtr data);
  [DllImport("user32.dll")] static extern bool EnumWindows(Visit visit, IntPtr data);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr window, out uint id);
  [DllImport("user32.dll")] static extern int GetClassName(IntPtr window, StringBuilder name, int size);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr window);
  public static bool Open(uint target) {
    bool found = false;
    EnumWindows((window, data) => {
      uint owner;
      GetWindowThreadProcessId(window, out owner);
      var name = new StringBuilder(64);
      GetClassName(window, name, 64);
      if (owner == target && name.ToString() == "#32770" && IsWindowVisible(window)) found = true;
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
'@
[Dialogs]::Open($ProcessId)
