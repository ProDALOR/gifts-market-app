import { useId } from "react";
import { palettes, setAppearance, useAppearance } from "../app/appearance";

export function AppearanceSettings() {
  const current = useAppearance();
  const id = useId();
  return (
    <section className="profile-card appearance-settings" aria-labelledby={id}>
      <h2 id={id}>Appearance</h2>
      <div
        className="palette-options"
        role="radiogroup"
        aria-label="Color palette"
      >
        {palettes.map((palette) => (
          <label key={palette.id}>
            <input
              type="radio"
              name={`${id}-palette`}
              value={palette.id}
              checked={current.palette === palette.id}
              onChange={() => setAppearance({ palette: palette.id })}
            />
            <span className="palette-option">
              <span
                className="palette-preview"
                data-palette={palette.id}
                aria-hidden="true"
              >
                <span />
                <span />
                <span />
              </span>
              <span>{palette.name}</span>
            </span>
          </label>
        ))}
      </div>
      <div
        className="display-mode-options"
        role="radiogroup"
        aria-label="Color mode"
      >
        {(["auto", "dark", "light"] as const).map((mode) => (
          <label key={mode}>
            <input
              type="radio"
              name={`${id}-mode`}
              value={mode}
              checked={current.mode === mode}
              onChange={() => setAppearance({ mode })}
            />
            <span>
              {mode === "auto" ? "Auto" : mode === "dark" ? "Dark" : "Light"}
            </span>
          </label>
        ))}
      </div>
      {current.mode === "auto" && <p>Follows Telegram or your device.</p>}
    </section>
  );
}
