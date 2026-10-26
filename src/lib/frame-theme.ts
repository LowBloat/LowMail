/**
 * Theme values for the documents shown in frames (a message being read, the
 * editor's body). Those documents cannot read the app's CSS custom
 * properties, so the tokens are resolved here, in the scheme asked for, and
 * written into their stylesheet.
 */

export interface FrameTheme {
	text: string;
	muted: string;
	hint: string;
	quote: string;
	link: string;
	surface: string;
	/** Body text size and typeface of the app. */
	font: string;
	family: string;
	stroke: string;
	/** Padding around the content: compact (the editor) and roomy (reading). */
	pad: { compact: [string, string]; roomy: [string, string] };
}

export function frameTheme(scheme: "light" | "dark"): FrameTheme {
	const probe = document.createElement("div");
	probe.dataset.themeScope = scheme;
	probe.setAttribute("aria-hidden", "true");
	Object.assign(probe.style, { position: "absolute", visibility: "hidden", pointerEvents: "none" });
	document.body.append(probe);
	const computed = getComputedStyle(probe);
	const color = (token: string) => {
		probe.style.color = `var(${token})`;
		return computed.color;
	};
	const length = (token: string) => {
		probe.style.width = `var(${token})`;
		return computed.width;
	};
	try {
		return {
			text: color("--ca-text-primary"),
			muted: color("--ca-text-secondary"),
			hint: color("--ca-text-hint"),
			quote: color("--ca-outline-default"),
			// Not --ca-brand-text: the LowBloat theme overrides it on :root, and
			// that override does not reach a light or dark scope.
			link: color("--ca-brand-main"),
			surface: color("--ca-surface-raised"),
			font: getComputedStyle(document.body).fontSize,
			family: getComputedStyle(document.body).fontFamily,
			stroke: length("--lm-stroke"),
			pad: {
				compact: [length("--ca-space-4-5"), length("--ca-space-6")],
				roomy: [length("--ca-space-6"), length("--ca-space-7")],
			},
		};
	} finally {
		probe.remove();
	}
}
