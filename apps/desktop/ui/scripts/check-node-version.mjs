const [major] = process.versions.node.split(".").map((v) => Number.parseInt(v, 10));

if (Number.isNaN(major) || major < 22 || major >= 23) {
  console.error(
    [
      "Unsupported Node.js version for Clawork UI build.",
      `Detected: ${process.versions.node}`,
      "Required: >=22 <23",
      "Reason: avoid intermittent Windows Vite/Rollup crash (exit -1073740791).",
    ].join("\n"),
  );
  process.exit(1);
}
