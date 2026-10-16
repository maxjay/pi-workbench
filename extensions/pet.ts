/**
 * Cues for Pi Workbench's pet, Glitch.
 *
 * - `/pet-say <text>` has it say something in its bubble.
 *
 * The workbench listens for the `pet` status line, which this sets for a
 * moment; other Pi clients just show it briefly in their footer. Any
 * extension can cue the pet the same way: `kill:<id>` (shot and thrown),
 * `say:<text>`, `wave` or `jump`.
 *
 * `/kill` itself belongs to the workbench: it shoots the pet and replaces
 * the session with a fresh one that keeps only what matters.
 *
 * Install: copy into ~/.pi/agent/extensions/ (or a project's
 * .pi/extensions/), then reload Pi.
 */
import type { ExtensionAPI, ExtensionContext } from "@earendil-works/pi-coding-agent";

/** Sets the `pet` status; the time makes every cue distinct. */
function cue(ctx: ExtensionContext, text: string) {
	// Held now: by the time the status clears, the session may have changed
	// and the context gone stale.
	const ui = ctx.ui;
	ui.setStatus("pet", text);
	setTimeout(() => {
		try {
			ui.setStatus("pet", undefined);
		} catch {
			// The session it belonged to has gone; nothing to clear.
		}
	}, 1500);
}

export default function (pi: ExtensionAPI) {
	pi.registerCommand("pet-say", {
		description: "Have the workbench's pet say something",
		handler: async (args, ctx) => {
			const text = (args ?? "").trim();
			if (text) cue(ctx, `say:${text}`);
		},
	});
}
