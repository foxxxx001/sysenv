# Local test server for sysenv http smoke tests.
$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://127.0.0.1:18899/")
$listener.Start()
Write-Output "SERVER_READY"
while ($listener.IsListening) {
    $ctx = $listener.GetContext()
    $req = $ctx.Request
    $resp = $ctx.Response
    $path = $req.Url.AbsolutePath
    $body = ""
    try {
        $reader = New-Object System.IO.StreamReader($req.InputStream, $req.ContentEncoding)
        $body = $reader.ReadToEnd()
    } catch {}
    switch ($path) {
        "/hello" {
            $resp.StatusCode = 200
            $resp.ContentType = "application/json"
            $json = @{ hello = "world"; method = $req.HttpMethod; path = $path } | ConvertTo-Json
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
            $resp.OutputStream.Write($bytes, 0, $bytes.Length)
        }
        "/echo" {
            $resp.StatusCode = 200
            $resp.ContentType = "application/json"
            $json = @{
                method = $req.HttpMethod
                body = $body
                contentType = $req.ContentType
                query = $req.Url.Query
                headerX = $req.Headers["X-Test"]
            } | ConvertTo-Json
            $bytes = [System.Text.Encoding]::UTF8.GetBytes($json)
            $resp.OutputStream.Write($bytes, 0, $bytes.Length)
        }
        "/status404" {
            $resp.StatusCode = 404
            $resp.ContentType = "text/plain"
            $bytes = [System.Text.Encoding]::UTF8.GetBytes("not found")
            $resp.OutputStream.Write($bytes, 0, $bytes.Length)
        }
        "/redirect" {
            $resp.StatusCode = 302
            $resp.RedirectLocation = "http://127.0.0.1:18899/hello"
            $resp.ContentLength64 = 0
            $resp.Close()
            continue
        }
        "/download.txt" {
            $resp.StatusCode = 200
            $resp.ContentType = "text/plain"
            $resp.AddHeader("Content-Disposition", "attachment; filename=`"report.txt`"")
            $bytes = [System.Text.Encoding]::UTF8.GetBytes("downloaded-content")
            $resp.OutputStream.Write($bytes, 0, $bytes.Length)
        }
        default {
            $resp.StatusCode = 404
            $bytes = [System.Text.Encoding]::UTF8.GetBytes("unknown")
            $resp.OutputStream.Write($bytes, 0, $bytes.Length)
        }
    }
    $resp.OutputStream.Close()
    $resp.Close()
}
