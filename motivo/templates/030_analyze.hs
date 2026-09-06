module Main (main) where

import Motivo.Method (Method (Analyze))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Analyze
