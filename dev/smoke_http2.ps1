$env:Path = "D:\rust\.cargo\bin;" + $env:Path
$bin = "D:\work\ai\systool\sysenv\target\debug\sysenv.exe"
$out = "D:\work\ai\systool\sysenv\smoke_http2.log"
"" | Out-File $out -Encoding utf8
function T($n, $script) {
    Add-Content $out "=== $n ===" -Encoding utf8
    $result = Invoke-Expression $script 2>&1
    Add-Content $out $result -Encoding utf8
    Add-Content $out "EXIT=$LASTEXITCODE" -Encoding utf8
    Add-Content $out "" -Encoding utf8
    Start-Sleep -Milliseconds 250
}
T "verbose -v GET" "& $bin http -v 127.0.0.1:18899/hello"
T "print=Hb" "& $bin http -p=Hb 127.0.0.1:18899/hello"
T "check-status 404" "& $bin http --check-status 127.0.0.1:18899/status404"
T "follow redirect" "& $bin http -F 127.0.0.1:18899/redirect"
T "no-follow redirect" "& $bin http 127.0.0.1:18899/redirect -b"
T "download -d" "& $bin http -d 127.0.0.1:18899/download.txt"
T "output -o file" "& $bin http -o D:\work\ai\systool\sysenv\out_body.json 127.0.0.1:18899/echo q==x"
T "offline" "& $bin http --offline POST 127.0.0.1:18899/echo a=1 b:=true"
T "nested JSON a[b][c]" "& $bin http POST 127.0.0.1:18899/echo a[b][c]=x a[list][]=1 a[list][]=2"
T "multipart upload" "& $bin http -f POST 127.0.0.1:18899/echo note=hi cv@D:\work\ai\systool\sysenv\test_body.json"
T "quiet -q" "& $bin http -q 127.0.0.1:18899/hello; Write-Output 'QUIET_RAN'"
T "raw --raw" "& $bin http POST 127.0.0.1:18899/echo --raw='{"x":1}'"
Add-Content $out "DONE" -Encoding utf8
