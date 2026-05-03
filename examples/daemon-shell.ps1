# daemon-shell.ps1 — PowerShell client for zshrs-daemon's HTTP service.
#
# Source from your $PROFILE:
#
#     . /path/to/daemon-shell.ps1
#     Daemon-Ping
#     Daemon-RecordAlias gst 'git status'
#     Daemon-DefsQuery -Kind alias -ShellId pwsh
#
# Endpoint and auth from env (set in $PROFILE):
#
#     $env:DAEMON_URL      = 'http://127.0.0.1:7733'
#     $env:DAEMON_TOKEN    = '<bearer-token>'        # optional
#     $env:DAEMON_SHELL_ID = 'pwsh'                  # see docs/SHELL_IDS.md
#
# Targets PowerShell 7+ (pwsh, cross-platform). Windows PowerShell 5.1
# is also supported — `Invoke-RestMethod` and the JSON cmdlets exist
# both places — but `pwsh` is recommended.
#
# All commands return parsed PSObjects (Invoke-RestMethod auto-decodes
# JSON), so you can pipe directly: `Daemon-Info | Select-Object version`.

if (-not $env:DAEMON_URL)      { $env:DAEMON_URL      = 'http://127.0.0.1:7733' }
if (-not $env:DAEMON_TOKEN)    { $env:DAEMON_TOKEN    = '' }
if (-not $env:DAEMON_SHELL_ID) { $env:DAEMON_SHELL_ID = 'pwsh' }

# ---- Internal helpers -----------------------------------------------------

function script:_DaemonHeaders {
    if ($env:DAEMON_TOKEN) {
        @{ 'Authorization' = "Bearer $($env:DAEMON_TOKEN)" }
    } else {
        @{}
    }
}

function script:_DaemonPost {
    param(
        [Parameter(Mandatory)] [string] $Op,
        [object] $Body = @{}
    )
    $jsonBody = if ($Body -is [string]) { $Body } else { $Body | ConvertTo-Json -Compress -Depth 10 }
    Invoke-RestMethod -Method POST `
        -Uri "$env:DAEMON_URL/op/$Op" `
        -Headers (_DaemonHeaders) `
        -ContentType 'application/json' `
        -Body $jsonBody
}

function script:_DaemonGet {
    param([Parameter(Mandatory)] [string] $Endpoint)
    Invoke-RestMethod -Uri "$env:DAEMON_URL$Endpoint" -Headers (_DaemonHeaders)
}

# ---- Public commands ------------------------------------------------------

function Daemon-Health { _DaemonGet '/health' }
function Daemon-Ops    { _DaemonGet '/ops' }
function Daemon-Info   { _DaemonPost 'info' @{} }

function Daemon-Ping {
    param([Parameter(ValueFromRemainingArguments)] [string[]] $EchoArgs)
    $body = if ($EchoArgs) { @{ echo = ($EchoArgs -join ' ') } } else { @{} }
    _DaemonPost 'ping' $body
}

function Daemon-Call {
    param(
        [Parameter(Mandatory)] [string] $Op,
        [hashtable] $Body = @{}
    )
    _DaemonPost $Op $Body
}

# ---- Federated recorder (definitions.*) ----------------------------------

function script:_DaemonEmit {
    param(
        [Parameter(Mandatory)] [string] $Kind,
        [Parameter(Mandatory)] [string] $Name,
        [string] $Value,
        [string] $File,
        [int]    $Line,
        [string] $FnChain
    )
    $body = @{
        shell_id = $env:DAEMON_SHELL_ID
        kind     = $Kind
        name     = $Name
    }
    if ($PSBoundParameters.ContainsKey('Value'))   { $body.value    = $Value }
    if ($PSBoundParameters.ContainsKey('File'))    { $body.file     = $File }
    if ($PSBoundParameters.ContainsKey('Line'))    { $body.line     = $Line }
    if ($PSBoundParameters.ContainsKey('FnChain')) { $body.fn_chain = $FnChain }
    _DaemonPost 'definitions_emit' $body
}

function Daemon-RecordAlias    { param([string]$Name, [string]$Body) _DaemonEmit -Kind alias    -Name $Name -Value $Body }
function Daemon-RecordGalias   { param([string]$Name, [string]$Body) _DaemonEmit -Kind galias   -Name $Name -Value $Body }
function Daemon-RecordSalias   { param([string]$Name, [string]$Body) _DaemonEmit -Kind salias   -Name $Name -Value $Body }
function Daemon-RecordFunction { param([string]$Name, [string]$Body) _DaemonEmit -Kind function -Name $Name -Value $Body }
function Daemon-RecordExport   { param([string]$Name, [string]$Value) _DaemonEmit -Kind env     -Name $Name -Value $Value }
function Daemon-RecordParam    { param([string]$Name, [string]$Value) _DaemonEmit -Kind params  -Name $Name -Value $Value }
function Daemon-RecordBindkey  { param([string]$Seq, [string]$Widget) _DaemonEmit -Kind bindkey -Name $Seq  -Value $Widget }
function Daemon-RecordCompdef  { param([string]$Cmd, [string]$Completer) _DaemonEmit -Kind compdef -Name $Cmd -Value $Completer }
function Daemon-RecordZstyle   { param([string]$Pattern, [string]$Style) _DaemonEmit -Kind zstyle -Name $Pattern -Value $Style }
function Daemon-RecordZmodload { param([string]$Module) _DaemonEmit -Kind zmodload -Name $Module }
function Daemon-RecordSetopt   { param([string]$Opt) _DaemonEmit -Kind setopt -Name $Opt -Value 'on' }
function Daemon-RecordUnsetopt { param([string]$Opt) _DaemonEmit -Kind setopt -Name $Opt -Value 'off' }
function Daemon-RecordSource   { param([string]$Path) _DaemonEmit -Kind source -Name $Path }
function Daemon-RecordPath     { param([string]$Dir) _DaemonEmit -Kind path -Name $Dir }
function Daemon-RecordFpath    { param([string]$Dir) _DaemonEmit -Kind fpath -Name $Dir }
function Daemon-RecordZle      { param([string]$Widget, [string]$Body = '') _DaemonEmit -Kind zle -Name $Widget -Value $Body }
function Daemon-RecordTrap     { param([string]$Signal, [string]$Handler) _DaemonEmit -Kind trap -Name $Signal -Value $Handler }
function Daemon-RecordNamedDir { param([string]$Name, [string]$Path) _DaemonEmit -Kind named_dir -Name $Name -Value $Path }
function Daemon-RecordCompletion { param([string]$Cmd, [string]$Path = '') _DaemonEmit -Kind completion -Name $Cmd -Value $Path }

# ---- Federated catalog query / diff --------------------------------------

function Daemon-DefsQuery {
    # NOTE: -Shell, not -ShellId. `$ShellId` is a PowerShell automatic
    # variable (read-only, holds the host ID like
    # "Microsoft.PowerShell") so a param named `[string] $ShellId`
    # raises "Cannot overwrite variable ShellId because it is read-only
    # or constant."
    param(
        [string] $Kind,
        [string] $Name,
        [string] $Prefix,
        [string] $Shell,
        [int]    $Limit
    )
    $body = @{}
    if ($PSBoundParameters.ContainsKey('Kind'))   { $body.kind     = $Kind }
    if ($PSBoundParameters.ContainsKey('Name'))   { $body.name     = $Name }
    if ($PSBoundParameters.ContainsKey('Prefix')) { $body.prefix   = $Prefix }
    if ($PSBoundParameters.ContainsKey('Shell'))  { $body.shell_id = $Shell }
    if ($PSBoundParameters.ContainsKey('Limit'))  { $body.limit    = $Limit }
    _DaemonPost 'definitions_query' $body
}

function Daemon-DefsKinds { _DaemonPost 'definitions_kinds' @{} }

function Daemon-DefsDiff {
    param(
        [Parameter(Mandatory)] [string] $ShellA,
        [Parameter(Mandatory)] [string] $ShellB,
        [string] $Kind
    )
    $body = @{ shell_a = $ShellA; shell_b = $ShellB }
    if ($PSBoundParameters.ContainsKey('Kind')) { $body.kind = $Kind }
    _DaemonPost 'definitions_diff' $body
}

# ---- Pubsub --------------------------------------------------------------
#
# SSE streaming endpoints (/stream/*) don't translate cleanly to
# Invoke-RestMethod's request/response model — use `curl -N` for
# Daemon-Watch / Daemon-Events from PowerShell, or `Invoke-WebRequest`
# with a custom buffered reader if you need a pure-PS solution.

function Daemon-Watch {
    param(
        [Parameter(Mandatory)] [string] $Dir,
        [switch] $Recursive
    )
    $rec = if ($Recursive) { 'true' } else { 'false' }
    & curl -sN -H "Authorization: Bearer $env:DAEMON_TOKEN" "$env:DAEMON_URL/stream/watch?path=$Dir&recursive=$rec"
}

function Daemon-Events {
    param([string] $Channel = '*.*')
    & curl -sN -H "Authorization: Bearer $env:DAEMON_TOKEN" "$env:DAEMON_URL/stream/events?channel=$Channel"
}

function Daemon-Publish {
    param(
        [Parameter(Mandatory)] [string] $Topic,
        [Parameter(Mandatory)] [object] $Data
    )
    _DaemonPost 'publish' @{ topic = $Topic; data = $Data }
}
