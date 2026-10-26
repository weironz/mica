param([string]$Signal)
$ErrorActionPreference = 'Stop'
Add-Type -TypeDefinition @'
using System;
using System.IO;
using System.Threading;
using System.Runtime.InteropServices;
public class MicaClipboardLock {
 [DllImport("user32.dll")] public static extern bool OpenClipboard(IntPtr hWnd);
 [DllImport("user32.dll")] public static extern bool CloseClipboard();
 [DllImport("user32.dll",CharSet=CharSet.Unicode)] public static extern IntPtr CreateWindowEx(int e,string c,string n,int s,int x,int y,int w,int h,IntPtr p,IntPtr m,IntPtr i,IntPtr q);
 [DllImport("user32.dll")] public static extern bool DestroyWindow(IntPtr h);
 public static void Hold(string path) {
  File.Delete(path+".release"); File.Delete(path+".ready");
  IntPtr hwnd=CreateWindowEx(0,"STATIC","Mica Clipboard Test",0,0,0,0,0,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero,IntPtr.Zero);
  if(hwnd==IntPtr.Zero) throw new Exception("CreateWindowEx failed");
  bool opened=false;
  for(int i=0;i<100;i++){if(OpenClipboard(hwnd)){opened=true;break;}Thread.Sleep(50);}
  if(!opened)throw new Exception("OpenClipboard failed");
  try{File.WriteAllText(path+".ready","locked");DateTime deadline=DateTime.UtcNow.AddMinutes(3);while(!File.Exists(path+".release")&&DateTime.UtcNow<deadline)Thread.Sleep(50);}
  finally{CloseClipboard();DestroyWindow(hwnd);File.Delete(path+".ready");}
 }
}
'@
[MicaClipboardLock]::Hold($Signal)
