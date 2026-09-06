module Main (main) where

import Motivo.Method (Method (Research))
import Motivo.Run (runMethod)

main :: IO ()
main = runMethod Research
