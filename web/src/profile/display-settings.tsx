import { useId } from "react";
import { setDisplayMode, useDisplayMode } from "../app/preferences";

export function DisplaySettings() {
  const mode = useDisplayMode();
  const id = useId();
  return (
    <section className="profile-card display-settings" aria-labelledby={id}>
      <h2 id={id}>Display</h2>
      <div
        className="display-mode-options"
        role="radiogroup"
        aria-labelledby={id}
      >
        {(["quality", "performance"] as const).map((value) => (
          <label key={value}>
            <input
              type="radio"
              name={id}
              value={value}
              checked={mode === value}
              onChange={() => setDisplayMode(value)}
            />
            <span>{value === "quality" ? "Quality" : "Performance"}</span>
          </label>
        ))}
      </div>
      <p>
        {mode === "quality"
          ? "Sharp cards and animations."
          : "Static cards; animate on hover or focus."}
      </p>
    </section>
  );
}
