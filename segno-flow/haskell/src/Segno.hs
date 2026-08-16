-- | Script-facing Segno DSL.  The typed Trigger/State/PersistentTask core is
-- implemented by Clef; this package supplies standard trigger/effect plugins
-- and the durable Haskell driver.
module Segno
  ( module Clef.Segno,
    module Segno.ActiveWindow,
    module Segno.Trigger.Time,
  )
where

import Clef.Segno
import Segno.ActiveWindow
import Segno.Trigger.Time
