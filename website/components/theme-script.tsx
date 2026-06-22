export function ThemeScript() {
  const code = `
    const stored = localStorage.getItem("onetcli-theme");
    const preferred = window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
    document.documentElement.dataset.theme = stored || preferred;
  `;
  return <script dangerouslySetInnerHTML={{ __html: code }} />;
}
