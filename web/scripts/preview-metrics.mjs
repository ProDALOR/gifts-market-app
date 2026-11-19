// Browser-side comparison helpers shared by the preview format and native-export probes.
export function installPreviewMetrics() {
  window.raster = async (url, size) => {
    const image = new Image();
    image.src = url;
    await image.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = size;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    context.drawImage(image, 0, 0, size, size);
    return { canvas, pixels: context.getImageData(0, 0, size, size).data };
  };
  // Compare on light and dark backgrounds, including transparency, instead of ignoring alpha.
  window.quality = (reference, actual) => {
    let error = 0,
      max = 0,
      changed = 0;
    for (let i = 0; i < reference.length; i += 4) {
      let pixelError = 0;
      for (const background of [24, 245])
        for (let c = 0; c < 3; c++) {
          const a =
            (reference[i + c] * reference[i + 3]) / 255 +
            background * (1 - reference[i + 3] / 255);
          const b =
            (actual[i + c] * actual[i + 3]) / 255 +
            background * (1 - actual[i + 3] / 255);
          const d = Math.abs(a - b);
          error += d * d;
          max = Math.max(max, d);
          pixelError = Math.max(pixelError, d);
        }
      if (pixelError > 10) changed++;
    }
    const mse = error / ((reference.length / 4) * 6);
    return {
      psnr: mse === 0 ? 100 : 10 * Math.log10((255 * 255) / mse),
      changedPercent: (changed / (reference.length / 4)) * 100,
      maxError: max,
    };
  };
}
