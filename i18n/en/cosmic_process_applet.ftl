app-title = Process Killer
popup-title = System Monitor
empty-list = No user processes found
loading = Loading…
kill = Kill
killing = Killing…
pid = PID { $pid }
process-count = { $count ->
    [one] 1 process
   *[other] { $count } processes
}
kill-failed = Could not kill { $name }: { $error }
scan-failed = Could not refresh processes: { $error }
