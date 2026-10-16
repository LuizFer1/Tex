#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6F2AD512-7ACC-433F-8B1F-54E9A97FFCBC}
AppName=Tex
AppVersion={#AppVersion}
AppPublisher=Luiz Fernando
AppPublisherURL=https://github.com/LuizFer1/Tex
DefaultDirName={autopf}\Tex
DisableProgramGroupPage=yes
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=..\target\installer
OutputBaseFilename=Tex-Setup-{#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\tex.exe
UsedUserAreasWarning=no

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ptbr"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[CustomMessages]
en.AccessTitle=Who can use Tex
en.AccessDescription=Choose which Windows accounts can search with Tex.
en.AccessIntro=The search service indexes the names of every file on every drive of this computer.
en.AccessNote=Everyone allowed here can see file names from all drives, including other accounts' files. If Windows asked for another account's password to install, "Only me" applies to that account, so choose "Everyone" instead.
en.AccessJustMe=Only me
en.AccessEveryone=Everyone who signs in to this computer
en.LaunchTex=Start Tex
en.ServiceFailed=Could not register the Tex search service (code %1). Search will not work until Tex is installed again.
en.RemoveData=Also delete the Tex search index and logs in %1?
ptbr.AccessTitle=Quem pode usar o Tex
ptbr.AccessDescription=Escolha quais contas do Windows podem buscar com o Tex.
ptbr.AccessIntro=O serviço de busca indexa o nome de todos os arquivos de todos os discos deste computador.
ptbr.AccessNote=Todos que tiverem acesso verão nomes de arquivos de todos os discos, inclusive de outras contas. Se o Windows pediu a senha de outra conta para instalar, "Apenas eu" vale para aquela conta; nesse caso escolha "Todos".
ptbr.AccessJustMe=Apenas eu
ptbr.AccessEveryone=Todos que entram neste computador
ptbr.LaunchTex=Abrir o Tex
ptbr.ServiceFailed=Não foi possível registrar o serviço de busca do Tex (código %1). A busca não vai funcionar até o Tex ser instalado de novo.
ptbr.RemoveData=Apagar também o índice de busca e os logs do Tex em %1?

[Files]
Source: "..\target\release\tex.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\tex-indexer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Tex"; Filename: "{app}\tex.exe"; Check: JustMe
Name: "{commonprograms}\Tex"; Filename: "{app}\tex.exe"; Check: Everyone

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Tex"; ValueData: """{app}\tex.exe"" --autostart"; Flags: uninsdeletevalue; Check: JustMe
Root: HKLM; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Tex"; ValueData: """{app}\tex.exe"" --autostart"; Flags: uninsdeletevalue; Check: Everyone

[Run]
Filename: "{app}\tex.exe"; Description: "{cm:LaunchTex}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM tex.exe"; Flags: runhidden waituntilterminated; RunOnceId: "CloseTex"
Filename: "{app}\tex-indexer.exe"; Parameters: "uninstall"; Flags: runhidden waituntilterminated; RunOnceId: "RemoveService"

[Code]
const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

var
  AccessPage: TInputOptionWizardPage;

function Everyone: Boolean;
begin
  Result := AccessPage.SelectedValueIndex = 1;
end;

function JustMe: Boolean;
begin
  Result := not Everyone;
end;

procedure InitializeWizard;
begin
  AccessPage := CreateInputOptionPage(wpSelectDir, CustomMessage('AccessTitle'),
    CustomMessage('AccessDescription'),
    CustomMessage('AccessIntro') + #13#10#13#10 + CustomMessage('AccessNote'), True, False);
  AccessPage.Add(CustomMessage('AccessJustMe'));
  AccessPage.Add(CustomMessage('AccessEveryone'));
  if GetPreviousData('Access', 'me') = 'everyone' then
    AccessPage.SelectedValueIndex := 1
  else
    AccessPage.SelectedValueIndex := 0;
end;

procedure RegisterPreviousData(PreviousDataKey: Integer);
begin
  if Everyone then
    SetPreviousData(PreviousDataKey, 'Access', 'everyone')
  else
    SetPreviousData(PreviousDataKey, 'Access', 'me');
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /IM tex.exe', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  ExtractTemporaryFile('tex-indexer.exe');
  Exec(ExpandConstant('{tmp}\tex-indexer.exe'), 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  Params: String;
  ResultCode: Integer;
begin
  if CurStep <> ssPostInstall then
    Exit;
  if Everyone then
  begin
    Params := 'install --all-users';
    RegDeleteValue(HKCU, RunKey, 'Tex');
    DeleteFile(ExpandConstant('{userprograms}\Tex.lnk'));
  end
  else
  begin
    Params := 'install';
    RegDeleteValue(HKLM, RunKey, 'Tex');
    DeleteFile(ExpandConstant('{commonprograms}\Tex.lnk'));
  end;
  if not Exec(ExpandConstant('{app}\tex-indexer.exe'), Params, '', SW_HIDE, ewWaitUntilTerminated, ResultCode)
    or (ResultCode <> 0) then
    SuppressibleMsgBox(FmtMessage(CustomMessage('ServiceFailed'), [IntToStr(ResultCode)]), mbError, MB_OK, IDOK);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Data: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;
  Data := ExpandConstant('{commonappdata}\Tex');
  if DirExists(Data) and not UninstallSilent
    and (MsgBox(FmtMessage(CustomMessage('RemoveData'), [Data]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
    DelTree(Data, True, True, True);
end;
