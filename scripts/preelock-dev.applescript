-- Preelock Dev: double-clickable launcher for the local dev build.
-- Starts the dev server + Tauri dev build in the background (no Terminal
-- window). Code changes hot-reload instantly (Vite HMR), exactly like
-- `npm run desktop`. Keep this app inside the Preelock repository so it can
-- locate scripts/preelock-dev-launcher.sh. See that script for details.
on run
	set appPath to POSIX path of (path to me)
	try
		set projectPath to do shell script "/usr/bin/git -C " & quoted form of appPath & " rev-parse --show-toplevel"
	on error
		display dialog "Keep Preelock Dev.app inside the Preelock repository so its launcher can be found." buttons {"OK"} default button 1
		return
	end try
	set launcherPath to projectPath & "/scripts/preelock-dev-launcher.sh"
	do shell script "nohup /bin/bash " & quoted form of launcherPath & " >/tmp/preelock-dev-launcher.log 2>&1 &"
end run
