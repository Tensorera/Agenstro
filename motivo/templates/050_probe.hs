module Main (main) where

import Motivo.Method (Method (Probe))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Probe
