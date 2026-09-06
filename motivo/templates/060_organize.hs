module Main (main) where

import Motivo.Method (Method (Organize))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Organize
