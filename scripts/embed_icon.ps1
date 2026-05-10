<#
.SYNOPSIS
  Windows API で ICO ファイルを PE 実行ファイルに埋め込む。
  CGO_ENABLED=0 環境でのアイコン注入に使用する。

.PARAMETER ExePath
  対象の実行ファイルパス

.PARAMETER IcoPath
  埋め込む ICO ファイルパス

.EXAMPLE
  .\scripts\embed_icon.ps1 -ExePath dist\ai-cli-hub.exe -IcoPath assets\ai-cli-hub.ico
#>
param(
    [Parameter(Mandatory)][string]$ExePath,
    [Parameter(Mandatory)][string]$IcoPath
)

$ErrorActionPreference = 'Stop'

$code = @"
using System;
using System.Runtime.InteropServices;
using System.IO;

public static class WinResEmbed {
    [DllImport("kernel32.dll", SetLastError=true, CharSet=CharSet.Unicode)]
    static extern IntPtr BeginUpdateResource(string pFileName, bool bDeleteExistingResources);

    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool UpdateResource(IntPtr hUpdate, IntPtr lpType, IntPtr lpName,
                                      ushort wLanguage, byte[] lpData, uint cb);

    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool EndUpdateResource(IntPtr hUpdate, bool fDiscard);

    const ushort RT_ICON = 3;
    const ushort RT_GROUP_ICON = 14;

    public static void Embed(string exePath, string icoPath) {
        byte[] ico = File.ReadAllBytes(icoPath);
        ushort count = BitConverter.ToUInt16(ico, 4);

        IntPtr h = BeginUpdateResource(exePath, false);
        if (h == IntPtr.Zero)
            throw new Exception("BeginUpdateResource failed: " + Marshal.GetLastWin32Error());

        // RT_GROUP_ICON ディレクトリ（6 + count*14 bytes）
        byte[] grp = new byte[6 + count * 14];
        // GRPICONDIR header: reserved(2), type=1(2), count(2)
        grp[2] = 1;
        BitConverter.GetBytes(count).CopyTo(grp, 4);

        for (int i = 0; i < count; i++) {
            int e = 6 + i * 16; // ICONDIRENTRY offset in ICO file
            byte   width      = ico[e];
            byte   height     = ico[e + 1];
            byte   colorCount = ico[e + 2];
            byte   reserved   = ico[e + 3];
            ushort planes     = BitConverter.ToUInt16(ico, e + 4);
            ushort bitCount   = BitConverter.ToUInt16(ico, e + 6);
            uint   byteSize   = BitConverter.ToUInt32(ico, e + 8);
            uint   imgOffset  = BitConverter.ToUInt32(ico, e + 12);

            byte[] img = new byte[byteSize];
            Array.Copy(ico, imgOffset, img, 0, byteSize);

            ushort id = (ushort)(i + 1);
            if (!UpdateResource(h, new IntPtr(RT_ICON), new IntPtr(id), 0, img, byteSize))
                throw new Exception("UpdateResource RT_ICON " + i + " failed: " + Marshal.GetLastWin32Error());

            int g = 6 + i * 14; // GRPICONDIR entry offset
            grp[g]     = width;
            grp[g + 1] = height;
            grp[g + 2] = colorCount;
            grp[g + 3] = reserved;
            BitConverter.GetBytes(planes).CopyTo(grp, g + 4);
            BitConverter.GetBytes(bitCount).CopyTo(grp, g + 6);
            BitConverter.GetBytes(byteSize).CopyTo(grp, g + 8);
            BitConverter.GetBytes(id).CopyTo(grp, g + 12);
        }

        if (!UpdateResource(h, new IntPtr(RT_GROUP_ICON), new IntPtr(1), 0, grp, (uint)grp.Length))
            throw new Exception("UpdateResource RT_GROUP_ICON failed: " + Marshal.GetLastWin32Error());

        if (!EndUpdateResource(h, false))
            throw new Exception("EndUpdateResource failed: " + Marshal.GetLastWin32Error());
    }
}
"@

if (-not ([System.Management.Automation.PSTypeName]'WinResEmbed').Type) {
    Add-Type -TypeDefinition $code -Language CSharp
}

$exeFull = (Resolve-Path $ExePath).Path
$icoFull = (Resolve-Path $IcoPath).Path

Write-Host "Embedding icon: $icoFull -> $exeFull"
[WinResEmbed]::Embed($exeFull, $icoFull)
Write-Host "Done."
