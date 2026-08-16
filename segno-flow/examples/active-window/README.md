# Active-window persistent task

`900_record_active_window.hs` is a deliberately small, model-free Segno task.
Its `time.interval` source plans one typed occurrence every 60 seconds, Segno
owns sleeping and cursor persistence, and the workflow:

1. reads the foreground-window title through the `system.active-window`
   plugin;
2. checkpoints a typed `WindowLog` through the `segno.state` SQLite plugin;
3. returns `Complete`, or an explicit five-second `Retry` after a CAS conflict.

Every successful checkpoint is retained in local business-state history. A
later workflow failure does not roll that checkpoint back. The example never
calls a provider or sends the window title over the network.

The built-in active-window plugin currently supports Windows. Segno's default
test suite uses a virtual clock and a fake task/process boundary, so it does
not wait for a real minute or read the desktop foreground state. CI also
type-checks this script as a separate Clef/Segno package integration check.
