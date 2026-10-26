/** New-mail notifications: native on the desktop, the Notification API in browsers. */
import { isDesktop } from "./api";

let asked = false;

export async function notify(title: string, body: string, onClick?: () => void) {
	if (typeof document !== "undefined" && document.hasFocus()) return;
	if (isDesktop) {
		const n = await import("@tauri-apps/plugin-notification");
		let granted = await n.isPermissionGranted();
		if (!granted) granted = (await n.requestPermission()) === "granted";
		if (granted) n.sendNotification({ title, body });
		return;
	}
	if (!("Notification" in window)) return;
	if (Notification.permission === "default" && !asked) {
		asked = true;
		await Notification.requestPermission();
	}
	if (Notification.permission !== "granted") return;
	const note = new Notification(title, { body, icon: "./favicon.svg", tag: "lowmail-new" });
	note.onclick = () => {
		window.focus();
		onClick?.();
		note.close();
	};
}

/** Asks for permission from a user gesture (the settings switch). */
export async function requestNotificationPermission(): Promise<boolean> {
	if (isDesktop) {
		const n = await import("@tauri-apps/plugin-notification");
		return (await n.isPermissionGranted()) || (await n.requestPermission()) === "granted";
	}
	if (!("Notification" in window)) return false;
	return (await Notification.requestPermission()) === "granted";
}
