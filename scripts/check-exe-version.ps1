param(
    # The Windows executable to check.
    [Parameter(Mandatory)][string]$Exe,
    # The expected package version (e.g. 0.4.0-alpha.8). Defaults to the
    # version of localcodepilot-desktop resolved by Cargo.
    [string]$Expected = ""
)

# Fails when the version resource of the executable does not match the package
# version (#99): FileVersion and ProductVersion must be the full version, and the
# numeric fields MAJOR.MINOR.PATCH.N, where N is the first number of the
# pre-release (0 without one), the same rule as desktop/build_support.rs.
$ErrorActionPreference = "Stop"

if (-not $Expected) {
    $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
    $Expected = ($metadata.packages | Where-Object { $_.name -eq "localcodepilot-desktop" }).version
}
if ($Expected -notmatch '^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$') {
    throw "Unsupported expected version: $Expected"
}
$core = "$($Matches[1]).$($Matches[2]).$($Matches[3])"
$preRelease = $Matches[4]
$fourth = 0
if ($preRelease) {
    $number = $preRelease.Split(".") | Where-Object { $_ -match '^\d+$' } | Select-Object -First 1
    if ($null -eq $number) { throw "Pre-release without a number: $Expected" }
    $fourth = [int]$number
}
$expectedNumbers = "$core.$fourth"

$info = (Get-Item -LiteralPath $Exe).VersionInfo
$found = [ordered]@{
    FileVersion = $info.FileVersion
    ProductVersion = $info.ProductVersion
    FileVersionRaw = "$($info.FileVersionRaw)"
    ProductVersionRaw = "$($info.ProductVersionRaw)"
}
$wanted = [ordered]@{
    FileVersion = $Expected
    ProductVersion = $Expected
    FileVersionRaw = $expectedNumbers
    ProductVersionRaw = $expectedNumbers
}
$wrong = @($wanted.Keys | Where-Object { $found[$_] -ne $wanted[$_] })
foreach ($field in $wanted.Keys) {
    Write-Host ("{0,-18} {1} (expected {2})" -f $field, $found[$field], $wanted[$field])
}
if ($wrong.Count -gt 0) {
    Write-Host "::error::$Exe does not carry version $Expected in: $($wrong -join ', ')"
    exit 1
}
Write-Host "$Exe carries version $Expected."
