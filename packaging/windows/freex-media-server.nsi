; Copyright (c) 2026 MirraNET, Niklas Linz. All rights reserved.
;
; This file is part of the MirraNET project and is licensed under the
; GNU Lesser General Public License v3.0 (LGPLv3).
;
; You may use, distribute and modify this code under the terms
; of the LGPLv3 license. You should have received a copy of the
; license along with this file. If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>
; or contact: niklas.linz@mirranet.de

; Windows installer of FreeX Media Server, built by build-installer.sh with:
;   makensis -DVERSION=0.12.2 -DFILEVERSION=0.12.2.0 -DSTAGE=<files to install> -DOUTFILE=<setup.exe>
;
; Installs the server with ffmpeg into Program Files and runs it as the Windows service
; "freex-media-server" (through WinSW). Configuration, data and logs live in
; ProgramData\FreeX Media Server, readable only by SYSTEM and administrators, and are kept on
; updates and (unless chosen otherwise) on uninstall.

Unicode true
ManifestDPIAware true

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

!define APP "FreeX Media Server"
!define SERVICE "freex-media-server"
!define PUBLISHER "MirraNET"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${SERVICE}"
!define DEFAULT_PORT "32500"

Name "${APP}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APP}"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
BrandingText "${APP} ${VERSION}"

VIProductVersion "${FILEVERSION}"
VIAddVersionKey /LANG=0 "ProductName" "${APP}"
VIAddVersionKey /LANG=0 "ProductVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileVersion" "${VERSION}"
VIAddVersionKey /LANG=0 "FileDescription" "${APP} Setup"
VIAddVersionKey /LANG=0 "CompanyName" "${PUBLISHER}"
VIAddVersionKey /LANG=0 "LegalCopyright" "${PUBLISHER}, LGPL-3.0"

!define MUI_ICON "freex.ico"
!define MUI_UNICON "freex.ico"
!define MUI_WELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_UNWELCOMEFINISHPAGE_BITMAP "welcome.bmp"
!define MUI_HEADERIMAGE
!define MUI_HEADERIMAGE_RIGHT
!define MUI_HEADERIMAGE_BITMAP "header.bmp"
!define MUI_ABORTWARNING

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "${STAGE}\LICENSE.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN
!define MUI_FINISHPAGE_RUN_TEXT "$(OpenWeb)"
!define MUI_FINISHPAGE_RUN_FUNCTION OpenWebInterface
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; The first language is used when the system's language is neither.
!insertmacro MUI_LANGUAGE "English"
!insertmacro MUI_LANGUAGE "German"

LangString OpenWeb ${LANG_ENGLISH} "Open FreeX in the browser"
LangString OpenWeb ${LANG_GERMAN} "FreeX im Browser öffnen"
LangString Needs64 ${LANG_ENGLISH} "${APP} needs a 64-bit version of Windows."
LangString Needs64 ${LANG_GERMAN} "${APP} benötigt eine 64-Bit-Version von Windows."
LangString StopService ${LANG_ENGLISH} "Stopping the running server..."
LangString StopService ${LANG_GERMAN} "Laufenden Server beenden..."
LangString StartService ${LANG_ENGLISH} "Starting the server..."
LangString StartService ${LANG_GERMAN} "Server starten..."
LangString ServiceFailed ${LANG_ENGLISH} "The Windows service could not be set up. The log is in $DataDir\logs."
LangString ServiceFailed ${LANG_GERMAN} "Der Windows-Dienst konnte nicht eingerichtet werden. Das Protokoll liegt in $DataDir\logs."
LangString ShortcutWeb ${LANG_ENGLISH} "FreeX"
LangString ShortcutWeb ${LANG_GERMAN} "FreeX"
LangString ShortcutData ${LANG_ENGLISH} "Configuration and logs"
LangString ShortcutData ${LANG_GERMAN} "Konfiguration und Protokolle"
LangString ShortcutUninstall ${LANG_ENGLISH} "Uninstall ${APP}"
LangString ShortcutUninstall ${LANG_GERMAN} "${APP} deinstallieren"
LangString DeleteData ${LANG_ENGLISH} "Also delete the configuration and all data (users, watch history, metadata)?$\r$\n$\r$\nYour media files are not touched."
LangString DeleteData ${LANG_GERMAN} "Auch die Konfiguration und alle Daten löschen (Benutzer, Verlauf, Metadaten)?$\r$\n$\r$\nDeine Mediendateien bleiben unberührt."

Var DataDir

!macro Service ARGS
    nsExec::ExecToLog '"$INSTDIR\freex-media-server-service.exe" ${ARGS}'
    Pop $0
!macroend

Function .onInit
    ${IfNot} ${RunningX64}
        MessageBox MB_ICONSTOP "$(Needs64)"
        Abort
    ${EndIf}
    SetRegView 64
    SetShellVarContext all
    ; Install into the folder of an earlier version, if any.
    ReadRegStr $0 HKLM "${UNINSTALL_KEY}" "InstallLocation"
    ${If} $0 != ""
        StrCpy $INSTDIR $0
    ${EndIf}
FunctionEnd

Function un.onInit
    SetRegView 64
    SetShellVarContext all
FunctionEnd

Section "Install"
    StrCpy $DataDir "$APPDATA\${APP}"

    ; An update: stop the service, so that its files can be replaced.
    ${If} ${FileExists} "$INSTDIR\freex-media-server-service.exe"
        DetailPrint "$(StopService)"
        !insertmacro Service "stop"
        !insertmacro Service "uninstall"
        Sleep 1000
    ${EndIf}
    ; ffmpeg is replaced as a whole, so no file of an old version is left behind.
    RMDir /r "$INSTDIR\ffmpeg"

    SetOutPath "$INSTDIR"
    File /r "${STAGE}\*.*"
    WriteUninstaller "$INSTDIR\uninstall.exe"

    ; Configuration and data: the server creates config.toml on its first start. Only SYSTEM (the
    ; service) and administrators may read them, as the database holds password hashes.
    CreateDirectory "$DataDir"
    nsExec::ExecToLog 'icacls "$DataDir" /inheritance:r /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F'
    Pop $0

    ; Allow access from other devices on home and company networks (not on public ones).
    nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="${APP}"'
    Pop $0
    nsExec::ExecToLog 'netsh advfirewall firewall add rule name="${APP}" dir=in action=allow program="$INSTDIR\freex-media-server.exe" enable=yes profile=private,domain'
    Pop $0

    DetailPrint "$(StartService)"
    !insertmacro Service "install"
    ${If} $0 != 0
        MessageBox MB_ICONEXCLAMATION "$(ServiceFailed)" /SD IDOK
    ${EndIf}
    !insertmacro Service "start"

    ; Start menu
    CreateDirectory "$SMPROGRAMS\${APP}"
    Call WriteWebShortcut
    CreateShortcut "$SMPROGRAMS\${APP}\$(ShortcutWeb).lnk" "$INSTDIR\FreeX.url" "" "$INSTDIR\freex.ico"
    CreateShortcut "$SMPROGRAMS\${APP}\$(ShortcutData).lnk" "$DataDir"
    CreateShortcut "$SMPROGRAMS\${APP}\$(ShortcutUninstall).lnk" "$INSTDIR\uninstall.exe"

    ; Apps & features
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayName" "${APP}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "Publisher" "${PUBLISHER}"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\freex.ico"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKLM "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
    WriteRegStr HKLM "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/FreeX-Studio/freex-media-server"
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKLM "${UNINSTALL_KEY}" "NoRepair" 1
SectionEnd

; The port from config.toml (the first "port = ..." line is server.port), or the default.
Function ReadPort
    StrCpy $1 "${DEFAULT_PORT}"
    ClearErrors
    FileOpen $2 "$DataDir\config.toml" r
    IfErrors done
    loop:
        FileRead $2 $3
        IfErrors close
        ; skip leading spaces
        StrCpy $4 0
        trim:
            StrCpy $5 $3 1 $4
            StrCmp $5 " " 0 +3
            IntOp $4 $4 + 1
            Goto trim
        StrCpy $3 $3 "" $4
        StrCpy $5 $3 4
        StrCmp $5 "port" 0 loop
        StrCpy $3 $3 "" 4
        ; skip spaces and the equals sign
        StrCpy $4 0
        trim2:
            StrCpy $5 $3 1 $4
            StrCmp $5 " " +2 0
            StrCmp $5 "=" 0 +3
            IntOp $4 $4 + 1
            Goto trim2
        StrCpy $3 $3 "" $4
        IntOp $3 $3 + 0
        ${If} $3 > 0
            StrCpy $1 $3
        ${EndIf}
    close:
        FileClose $2
    done:
FunctionEnd

Function WriteWebShortcut
    Call ReadPort
    WriteINIStr "$INSTDIR\FreeX.url" "InternetShortcut" "URL" "http://localhost:$1/"
    WriteINIStr "$INSTDIR\FreeX.url" "InternetShortcut" "IconFile" "$INSTDIR\freex.ico"
    WriteINIStr "$INSTDIR\FreeX.url" "InternetShortcut" "IconIndex" "0"
FunctionEnd

Function OpenWebInterface
    ; The server needs a moment for its first start (it then also writes config.toml).
    Sleep 3000
    Call WriteWebShortcut
    ExecShell "open" "http://localhost:$1/"
FunctionEnd

Section "Uninstall"
    StrCpy $DataDir "$APPDATA\${APP}"
    !insertmacro Service "stop"
    !insertmacro Service "uninstall"
    Sleep 1000
    nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="${APP}"'
    Pop $0

    RMDir /r "$SMPROGRAMS\${APP}"
    RMDir /r "$INSTDIR"
    DeleteRegKey HKLM "${UNINSTALL_KEY}"

    MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "$(DeleteData)" /SD IDNO IDNO keep
        RMDir /r "$DataDir"
    keep:
SectionEnd
