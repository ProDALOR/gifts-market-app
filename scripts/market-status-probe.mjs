// Read the app-facing availability event without credentials or operational endpoints.
const origin = process.argv[2];
if (!origin) throw new Error("Provide the app URL");
const socket = new WebSocket(
  new URL("/ws", origin).href.replace(/^http/, "ws"),
);
let timer;
try {
  const status = await new Promise((resolve, reject) => {
    timer = setTimeout(
      () => reject(new Error("Market status timed out")),
      5000,
    );
    socket.addEventListener("error", () =>
      reject(new Error("Market status connection failed")),
    );
    socket.addEventListener("close", () =>
      reject(new Error("Market status connection closed")),
    );
    socket.addEventListener("message", (event) => {
      try {
        const frame = JSON.parse(event.data);
        if (
          frame.type === "market.status" &&
          ["available", "delayed", "unavailable"].includes(frame.status)
        )
          resolve(frame.status);
      } catch (error) {
        reject(error);
      }
    });
  });
  console.log(status);
} finally {
  clearTimeout(timer);
  socket.close();
}
