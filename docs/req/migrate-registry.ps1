#!/usr/bin/env pwsh
# Idempotent registry schema normalization for docs/req/registry.csv.
# SEPARATE changeset: this transform must not be mixed with semantic/algorithm changes.
# Routing/algorithm meaning is unchanged before and after; only column schema is normalized.
#
# Target schema (docs/req/README.md):
#   area   in CORE|TOOL|ROUTE|SPATIAL|BIND|STUDIO|OPS|QAL   (codebase.md R7)
#   phase  integer 1..34 (work-plan); legacy area-name resolved from the owning algorithm
#          doc's declared "Phase N" / "Phases N, M" / "Phases N-M". A row whose owning docs
#          yield more than one or no phase is FLAGGED for manual resolution, never guessed.
#   status in open|specified|implemented|verified
#   id     unique; every REQ cited under docs/ exists in the registry
#
# Usage:
#   pwsh migrate-registry.ps1 -Check   # validate only, no write
#   pwsh migrate-registry.ps1 -Report  # list flagged rows, no write
#   pwsh migrate-registry.ps1          # write normalized registry (refuses if any row unresolvable)
[CmdletBinding()]
param([switch]$Check, [switch]$Report)

$ErrorActionPreference = 'Stop'
$regPath = Join-Path $PSScriptRoot 'registry.csv'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$docsDir = (Resolve-Path (Join-Path $PSScriptRoot '..\..\docs')).Path

$Areas    = 'CORE','TOOL','ROUTE','SPATIAL','BIND','STUDIO','OPS','QAL'
$Statuses = 'open','specified','implemented','verified'

function Get-DeclaredPhases([string[]]$lines) {
  foreach ($line in ($lines | Select-Object -First 8)) {
    if ($line -match 'Phases?\s+([0-9][^.;]*)') {
      $seg = $Matches[1]
      $ph = New-Object System.Collections.Generic.List[int]
      foreach ($m in [regex]::Matches($seg, '(\d+)\s*[\u2013\u2014-]\s*(\d+)')) {
        for ($i = [int]$m.Groups[1].Value; $i -le [int]$m.Groups[2].Value; $i++) {
          if ($i -ge 1 -and $i -le 34) { $ph.Add($i) }
        }
      }
      foreach ($m in [regex]::Matches($seg, '\d+')) {
        $v = [int]$m.Value; if ($v -ge 1 -and $v -le 34) { $ph.Add($v) }
      }
      $uniq = $ph | Sort-Object -Unique
      if ($uniq) { return @($uniq) }
    }
  }
  return @()
}

# 1. doc path (as cited in source_ref) -> declared phases.
$docPhase = @{}
Get-ChildItem $docsDir -Recurse -Filter *.md | ForEach-Object {
  $rel = ($_.FullName.Substring($repoRoot.Length + 1)) -replace '\\','/'
  $p = Get-DeclaredPhases (Get-Content $_.FullName)
  if ($p.Count) { $docPhase[$rel] = $p }
}

$lines = [System.IO.File]::ReadAllLines($regPath)
$header = $lines[0]
$rows = Import-Csv $regPath
$problems = New-Object System.Collections.Generic.List[string]
$flagged  = New-Object System.Collections.Generic.List[string]
$resolved = New-Object System.Collections.Generic.List[object]

foreach ($r in $rows) {
  $area = $r.area; $status = $r.status; $phase = $r.phase

  if ($Areas -notcontains $area)      { $problems.Add("BAD area '$area' id=$($r.id)") }
  if ($Statuses -notcontains $status) { $problems.Add("BAD status '$status' id=$($r.id)") }

  if ($phase -match '^\d+$') {
    if ([int]$phase -lt 1 -or [int]$phase -gt 34) { $problems.Add("OUT-OF-RANGE phase '$phase' id=$($r.id)") }
  }
  else {
    $cand = New-Object System.Collections.Generic.List[int]
    foreach ($p in ($r.source_ref -split ';' | ForEach-Object { $_.Trim() })) {
      if ($docPhase.ContainsKey($p)) { foreach ($v in $docPhase[$p]) { $cand.Add($v) } }
    }
    $cand = @($cand | Sort-Object -Unique)
    if ($cand.Count -eq 1) { $phase = [string]$cand[0] }
    else {
      $flagged.Add("FLAG phase '$($r.phase)' id=$($r.id) candidates=[$($cand -join ',')] refs='$($r.source_ref)'")
      $problems.Add("UNRESOLVED phase '$($r.phase)' id=$($r.id) candidates=[$($cand -join ',')]")
      $phase = $r.phase
    }
  }

  $resolved.Add([pscustomobject]@{ id=$r.id; area=$area; statement=$r.statement; source_ref=$r.source_ref; phase=$phase; test=$r.test; status=$status })
}

# 2. uniqueness + dangling citations.
$dupes = $resolved | Group-Object id | Where-Object Count -gt 1
foreach ($d in $dupes) { $problems.Add("DUPLICATE id $($d.Name) x$($d.Count)") }

$present = @{}; $resolved | ForEach-Object { $present[$_.id] = $true }
$cited = @{}
Get-ChildItem $docsDir -Recurse -Include *.md,*.csv | Select-String -Pattern 'REQ-[A-Z]+-\d+' -AllMatches |
  ForEach-Object { $_.Matches.Value } | ForEach-Object { $cited[$_] = $true }
foreach ($c in ($cited.Keys | Where-Object { -not $present.ContainsKey($_) })) { $problems.Add("DANGLING citation $c") }

"rows=$($resolved.Count) unique=$(($resolved | Select-Object -ExpandProperty id | Sort-Object -Unique).Count) problems=$($problems.Count) flagged=$($flagged.Count)"
$flagged | ForEach-Object { "  $_" }
if (-not $Report) { $problems | ForEach-Object { "  $_" } }

if ($Check -or $Report) { if ($problems.Count) { exit 1 } else { exit 0 } }
if ($problems.Count) { Write-Error "refusing to write: resolve the rows above (never guess a phase)"; exit 1 }

$out = foreach ($r in $resolved) {
  '{0},{1},"{2}","{3}",{4},{5},{6}' -f $r.id, $r.area, ($r.statement -replace '"','""'), $r.source_ref, $r.phase, $r.test, $r.status
}
[System.IO.File]::WriteAllLines($regPath, @($header) + $out)
"wrote normalized registry: $($out.Count) rows"
