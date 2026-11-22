// Applies the stored theme and mode before first paint, so a theme never flashes.
const root = document.documentElement;
root.dataset.theme = localStorage.getItem("priory.theme") || "graphite";
root.dataset.mode = localStorage.getItem("priory.mode") || "system";
