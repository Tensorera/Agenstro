module Main (main) where

import Motivo.Method (Method (Investigate))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Investigate
