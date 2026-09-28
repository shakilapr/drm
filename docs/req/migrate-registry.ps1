#!/usr/bin/env pwsh
# Idempotent registry schema normalization for docs/req/registry.csv.
# SEPARATE changeset: this transform must not be mixed with semantic/algorithm changes.
# Routing/algorithm meaning is unchanged before and after; only column schema is normalized.
#
# Target schema (docs/req/README.md):
#   area   in CORE|TOOL|ROUTE|SPATIAL|BIND|STUDIO|OPS|QAL   (codebase.md R7)
#   phase  integer 1..34 (work-plan); legacy area-name resolved from the owning doc's "Phase N" header
#   status in open|specified|implemented|verified
#   id     unique; every REQ cited under docs/ exists in the registry
#
# Usage:
#   pwsh migrate-registry.ps1 -Check   # validate only, no write
#   pwsh migrate-registry.ps1          # write normalized registry (refuses if any row unresolvable)
[CmdletBinding()]
param([switch]$Check)

$ErrorActionPreference = 'Stop'
$regPath = Join-Path $PSScriptRoot 'registry.csv'
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$docsDir = (Resolve-Path (Join-Path $PSScriptRoot '..\..\docs')).Path

$Areas    = 'CORE','TOOL','ROUTE','SPATIAL','BIND','STUDIO','OPS','QAL'
$Statuses = 'open','specified','implemented','verified'

# 1. doc path -> declared phase, from each algorithm/doc header ("Phase 29").
$docPhase = @{}
Get-ChildItem $docsDir -Recurse -Filter *.md | ForEach-Object {
  $rel = ($_.FullName.Substring($repoRoot.Length + 1)) -replace '\\','/'
  foreach ($line in (Get-Content $_.FullName -TotalCount 6)) {
    if ($line -match 'Phase\s+(\d{1,2})') { $docPhase[$rel] = [int]$Matches[1]; break }
  }
}

$lines = [System.IO.File]::ReadAllLines($regPath)
$header = $lines[0]
$rows = Import-Csv $regPath
$problems = New-Object System.Collections.Generic.List[string]
$resolved = New-Object System.Collections.Generic.List[object]

foreach ($r in $rows) {
  $area = $r.area; $status = $r.status; $phase = $r.phase

  if ($Areas -notcontains $area)      { $problems.Add("BAD area '$area' id=$($r.id)") }
  if ($Statuses -notcontains $status) { $problems.Add("BAD status '$status' id=$($r.id)") }

  if ($phase -notmatch '^\d+$') {
    $cand = @()
    foreach ($p in ($r.source_ref -split ';' | ForEach-Object { $_.Trim() })) {
      if ($docPhase.ContainsKey($p)) { $cand += $docPhase[$p] }
    }
    $cand = $cand | Sort-Object -Unique
    if ($cand.Count -eq 1) { $phase = [string]$cand[0] }
    else { $problems.Add("UNRESOLVED phase '$($r.phase)' id=$($r.id) refs='$($r.source_ref)'"); $phase = $r.phase }
  }
  elseif ([int]$phase -lt 1 -or [int]$phase -gt 34) { $problems.Add("OUT-OF-RANGE phase '$phase' id=$($r.id)") }

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

"rows=$($resolved.Count) unique=$(($resolved | Select-Object -ExpandProperty id | Sort-Object -Unique).Count) problems=$($problems.Count)"
$problems | ForEach-Object { "  $_" }

if ($Check) { if ($problems.Count) { exit 1 } else { exit 0 } }
if ($problems.Count) { Write-Error "refusing to write: resolve the rows above (never guess a phase)"; exit 1 }

$out = foreach ($r in $resolved) {
  '{0},{1},"{2}","{3}",{4},{5},{6}' -f $r.id, $r.area, ($r.statement -replace '"','""'), $r.source_ref, $r.phase, $r.test, $r.status
}
[System.IO.File]::WriteAllLines($regPath, @($header) + $out)
"wrote normalized registry: $($out.Count) rows"
