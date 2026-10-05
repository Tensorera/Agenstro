{-# LANGUAGE OverloadedStrings #-}

module Main (main, verifyImplementation) where

import Clef (Workflow, requireBecause)
import Data.Text (Text)
import Motivo.Test

acceptance :: Suite
acceptance = suite "addition"
  [ testCase "two-plus-two"
      (withTimeout 15 (command ["/usr/bin/python3", "/project/add.py", "2", "2"]))
      [assertExit 0, assertStdout "4\n"],
    testCase "negative-operand"
      (withTimeout 15 (command ["/usr/bin/python3", "/project/add.py", "-3", "5"]))
      [assertExit 0, assertStdout "2\n"]
  ]

-- An existing provider workflow may call this after producing its implementation.
-- The actual commands decide acceptance, independently of testsRun in a reply.
verifyImplementation :: FilePath -> Text -> Workflow SuiteResult
verifyImplementation root runId = do
  actual <- runSuite root runId acceptance
  requireBecause "Independent addition acceptance failed" suitePassed actual

main :: IO ()
main = runTestSuite acceptance
