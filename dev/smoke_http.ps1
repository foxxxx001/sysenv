$env:Path = "D:\rust\.cargo\bin;" + $env:Path
$bin = "D:\work\ai\systool\sysenv\target\debug\sysenv.exe"
$out = "D:\work\ai\systool\sysenv\smoke_http.log"
"" | Out-File $out -Encoding utf8
function T($n, $script) {
    Add-Content $out "=== $n ===" -Encoding utf8
    $result = Invoke-Expression $script 2>&1
    Add-Content $out $result -Encoding utf8
    Add-Content $out "EXIT=$LASTEXITCODE" -Encoding utf8
    Add-Content $out "" -Encoding utf8
    Start-Sleep -Milliseconds 250
}
T "GET default" "& $bin http 127.0.0.1:18899/hello"
T "GET -b" "& $bin http -b 127.0.0.1:18899/hello"
T "POST JSON name=John age:=29" "& $bin http POST 127.0.0.1:18899/echo name=John age:=29"
T "GET query q==a per_page==2" "& $bin http 127.0.0.1:18899/echo q==a per_page==2"
T "POST form -f" "& $bin http -f POST 127.0.0.1:18899/echo name='John Smith'"
T "header item X-Test:abc" "& $bin http 127.0.0.1:18899/echo X-Test:abc"
T "POST raw body @file" "& $bin http POST 127.0.0.1:18899/echo @D:\work\ai\systool\sysenv\test_body.json"
Add-Content $out "DONE" -Encoding utf8
