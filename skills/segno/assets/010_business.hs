module Main (main) where

-- Install at .tactus/scripts/010_business.hs. Replace this model-free
-- operation with the requested business workflow while retaining its args.
import System.Directory (createDirectoryIfMissing)
import System.Environment (getArgs)
import System.FilePath ((</>))

main :: IO ()
main = do
  arguments <- getArgs
  case arguments of
    [workDirectory] -> do
      createDirectoryIfMissing True workDirectory
      writeFile (workDirectory </> "report.txt") "Business workflow completed.\n"
      putStrLn ("Report: " <> workDirectory </> "report.txt")
    _ -> fail "usage: 010_business.hs WORK_DIRECTORY"
