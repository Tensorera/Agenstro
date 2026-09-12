{-# LANGUAGE OverloadedStrings #-}
{-# LANGUAGE ScopedTypeVariables #-}

-- | A small read-only, loopback-only status surface. No provider credentials,
-- arbitrary file serving, mutation routes, or external assets are involved.
module Segno.Service.Web (withServiceWeb) where

import Control.Concurrent.Async (race_)
import Control.Exception (IOException, bracket, catch)
import Control.Monad (forever, unless, void)
import Data.Aeson (encode, object, (.=))
import qualified Data.ByteString as BS
import qualified Data.ByteString.Char8 as Char8
import qualified Data.ByteString.Lazy as LBS
import Data.Time (getCurrentTime)
import Network.Socket
  ( Family (AF_INET), Socket, SocketOption (ReuseAddr), SocketType (Stream), SockAddr (SockAddrInet),
    accept, bind, close, defaultProtocol, listen, setSocketOption, socket, tupleToHostAddress, withSocketsDo )
import Network.Socket.ByteString (recv, sendAll)
import Segno.Service.Registry (listServiceJobs, serviceRunHistory)
import System.Timeout (timeout)

-- | The listener is bound before scheduling starts, so a port collision cannot
-- accidentally start task execution. A bounded request timeout limits slow peers.
withServiceWeb :: FilePath -> Int -> IO () -> IO ()
withServiceWeb registry port scheduler = do
  unless (port >= 1024 && port <= 65535) (fail "port must be in 1024..65535")
  withSocketsDo $ bracket openListener close $ \listener ->
    race_ scheduler (forever (serveClient listener))
  where
    openListener = do
      listener <- socket AF_INET Stream defaultProtocol
      (do
        setSocketOption listener ReuseAddr 1
        bind listener (SockAddrInet (fromIntegral port) (tupleToHostAddress (127, 0, 0, 1)))
        listen listener 16
        pure listener) `catch` (\(exception :: IOException) -> close listener >> ioError exception)
    serveClient listener =
      (bracket (fst <$> accept listener) close $ \client ->
        void (timeout (5 * 1000000) (respond registry port client))) `catch` (\(_ :: IOException) -> pure ())

respond :: FilePath -> Int -> Socket -> IO ()
respond registry port client = do
  request <- readHeaders client BS.empty
  let rows = Char8.lines request
      first = case rows of [] -> []; line : _ -> Char8.words line
      headers = map (Char8.map toLowerAscii . stripCR) (drop 1 rows)
      hosts = [Char8.dropWhile (== ' ') (BS.drop 5 line) | line <- headers, "host:" `BS.isPrefixOf` line]
      validHost = hosts == [Char8.pack ("127.0.0.1:" <> show port)] || hosts == [Char8.pack ("localhost:" <> show port)]
      crossSite = any (`elem` ["sec-fetch-site: cross-site", "sec-fetch-site: same-site"]) headers
  if not validHost || crossSite
    then reply client "403 Forbidden" "text/plain; charset=utf-8" "Local requests only"
    else case first of
      ["GET", "/", "HTTP/1.1"] -> reply client "200 OK" "text/html; charset=utf-8" page
      ["GET", "/api/status", "HTTP/1.1"] -> do
        jobs <- listServiceJobs registry
        runs <- serviceRunHistory registry 100
        now <- getCurrentTime
        reply client "200 OK" "application/json; charset=utf-8" (LBS.toStrict (encode (object ["api" .= ("agenstro.segno-service-status/v1" :: String), "observed_at" .= now, "jobs" .= jobs, "runs" .= runs])))
      _ -> reply client "404 Not Found" "text/plain; charset=utf-8" "Not found"
  where
    toLowerAscii c | c >= 'A' && c <= 'Z' = toEnum (fromEnum c + 32)
                   | otherwise = c
    stripCR = Char8.takeWhile (/= '\r')

readHeaders :: Socket -> BS.ByteString -> IO BS.ByteString
readHeaders client accumulated
  | BS.length accumulated > 8192 = pure BS.empty
  | "\r\n\r\n" `BS.isInfixOf` accumulated = pure accumulated
  | otherwise = do
      chunk <- recv client 2048
      if BS.null chunk then pure accumulated else readHeaders client (accumulated <> chunk)

reply :: Socket -> BS.ByteString -> BS.ByteString -> BS.ByteString -> IO ()
reply client status contentType body = sendAll client $
  "HTTP/1.1 " <> status <> "\r\nContent-Type: " <> contentType <>
  "\r\nContent-Length: " <> Char8.pack (show (BS.length body)) <>
  "\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n" <>
  "Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'\r\n\r\n" <> body

page :: BS.ByteString
page = Char8.unlines
  [ "<!doctype html><html lang=en><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>Segno tasks</title>",
    "<style>body{font:16px system-ui;margin:3rem auto;max-width:1100px;padding:0 1rem;color:#19332b;background:#f4f7f3}h1{margin-bottom:.3rem}p{color:#52665d}table{width:100%;border-collapse:collapse;background:white;margin:1rem 0 2rem}th,td{text-align:left;padding:.7rem;border-bottom:1px solid #dee6df;vertical-align:top;overflow-wrap:anywhere}td pre{white-space:pre-wrap;max-width:35rem;margin:0;font:13px monospace}.bad{color:#ad3c20}.good{color:#21704c}</style>",
    "<h1>Segno</h1><p>Persistent tasks on this machine. Refreshes every 2 seconds.</p><p id=connection>Connecting...</p>",
    "<h2>Registered workspaces</h2><table><thead><tr><th>Task</th><th>Workspace</th><th>Schedule</th><th>Next invocation</th></tr></thead><tbody id=jobs></tbody></table>",
    "<h2>Recent invocations</h2><table><thead><tr><th>Task / timestamp</th><th>State</th><th>Result</th></tr></thead><tbody id=runs></tbody></table>",
    "<script>const cell=(row,value)=>{const c=document.createElement('td');c.textContent=value;row.append(c);return c};async function refresh(){try{const response=await fetch('/api/status',{cache:'no-store'});if(!response.ok)throw Error('HTTP '+response.status);const data=await response.json();document.getElementById('connection').textContent='Updated '+new Date(data.observed_at).toLocaleString();const jobs=document.getElementById('jobs');jobs.replaceChildren();for(const j of data.jobs){const row=document.createElement('tr');cell(row,j.name);cell(row,j.root);cell(row,j.enabled?'Every '+j.every_seconds+' seconds':'Paused');cell(row,j.next_at);jobs.append(row)}const runs=document.getElementById('runs');runs.replaceChildren();for(const r of data.runs){const row=document.createElement('tr');cell(row,r.job+' / '+r.timestamp);const state=cell(row,r.state);state.className=r.state==='succeeded'?'good':r.state==='running'?'':'bad';const details=document.createElement('pre');details.textContent=JSON.stringify({status_code:r.status_code,info:r.info},null,2);cell(row,'').append(details);runs.append(row)}}catch(error){document.getElementById('connection').textContent='Disconnected: '+error.message}finally{setTimeout(refresh,2000)}}refresh();</script></html>"
  ]
