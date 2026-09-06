module Main (main) where

import Motivo.Method (Method (Clarify))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Clarify
