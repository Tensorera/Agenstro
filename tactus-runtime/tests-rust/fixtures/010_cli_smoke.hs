module Main (main) where

import Clef (runTactus)

main :: IO ()
main = runTactus (pure ("TACTUS_RUN_OK" :: String)) >>= putStrLn
