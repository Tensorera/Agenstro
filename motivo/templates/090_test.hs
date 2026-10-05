-- Run a user-authored JSON suite. Use --help for red/green and baseline options.
import Motivo.Test (runTestSuiteFile)

main :: IO ()
main = runTestSuiteFile
