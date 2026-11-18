import { useEffect, useState } from "react";
import { micromark } from "micromark";
import { math, mathHtml } from "micromark-extension-math";
import { ResourceCache } from "../resources/cache";
import { guideKey } from "../valuation/strategies";
import type { ValuationStrategy } from "../market";

// Cache rendered MathML as well as the download, keyed by the server's content revision.
const guides = new ResourceCache(
  async (key, signal) => {
    const [url] = JSON.parse(key) as [string, string];
    const response = await fetch(url, { signal });
    if (
      !response.ok ||
      !response.headers.get("content-type")?.startsWith("text/markdown")
    )
      throw new Error(
        "The calculation guide is unavailable. Please try again.",
      );
    const markdown = await response.text();
    signal.throwIfAborted();
    const html = micromark(markdown, {
      extensions: [math()],
      htmlExtensions: [
        mathHtml({
          output: "mathml",
          trust: false,
          throwOnError: false,
          strict: "error",
        }),
      ],
      allowDangerousHtml: false,
      allowDangerousProtocol: false,
    });
    return { value: html, bytes: html.length * 2 };
  },
  512 * 1024,
  2,
);

/** Loaded only after expansion. The API serves the strategy's own Markdown file. */
export default function ValuationMethodology({
  strategy,
}: {
  strategy: ValuationStrategy;
}) {
  const key = guideKey(strategy);
  const [attempt, retry] = useState(0);
  const [state, setState] = useState<{ html?: string; error?: string }>({});
  useEffect(() => {
    const stop = new AbortController();
    setState({});
    void guides
      .get(key, stop.signal)
      .then((html) => {
        if (!stop.signal.aborted) setState({ html });
      })
      .catch((error) => {
        if (!stop.signal.aborted)
          setState({
            error:
              error instanceof Error
                ? error.message
                : "The calculation guide is unavailable.",
          });
      });
    return () => stop.abort();
  }, [key, attempt]);
  if (state.error)
    return (
      <div className="valuation-guide-error" role="alert">
        <p>{state.error}</p>
        <button onClick={() => retry((n) => n + 1)}>Retry guide</button>
      </div>
    );
  if (state.html === undefined)
    return (
      <p className="valuation-note" role="status">
        Loading calculation guide…
      </p>
    );
  return (
    <article
      className="valuation-guide-content"
      aria-label={`${strategy.name} calculation guide`}
      dangerouslySetInnerHTML={{ __html: state.html }}
    />
  );
}
