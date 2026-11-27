import { BrandArtwork } from "../brand/brand-mark";

export function LoadingScreen() {
  return (
    <main
      className="workspace startup-loading"
      role="status"
      aria-label="Loading app"
    >
      <BrandArtwork animated />
    </main>
  );
}
