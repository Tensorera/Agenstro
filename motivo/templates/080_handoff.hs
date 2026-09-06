module Main (main) where

import Motivo.Method (Method (Handoff))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Handoff
