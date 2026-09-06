module Main (main) where

import Motivo.Method (Method (Retrospect))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Retrospect
