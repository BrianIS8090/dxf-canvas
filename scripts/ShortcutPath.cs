using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
using System.Text;

public static class DxfShortcut
{
  // WScript.Shell читает путь через ANSI на старых Windows и теряет кириллицу.
  [ComImport, Guid("000214F9-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
  private interface IShellLinkW
  {
    [PreserveSig]
    int GetPath([Out, MarshalAs(UnmanagedType.LPWStr)] StringBuilder path,
      int capacity, IntPtr findData, uint flags);
  }

  public static string Read(string shortcut)
  {
    var type = Type.GetTypeFromCLSID(new Guid("00021401-0000-0000-C000-000000000046"));
    var instance = Activator.CreateInstance(type);
    try
    {
      ((IPersistFile)instance).Load(shortcut, 0);
      var path = new StringBuilder(32768);
      Marshal.ThrowExceptionForHR(((IShellLinkW)instance).GetPath(path, path.Capacity, IntPtr.Zero, 4));
      return path.ToString();
    }
    finally
    {
      Marshal.FinalReleaseComObject(instance);
    }
  }
}
