import { createRoot } from "react-dom/client";
import EasyTierDialog from "../../../src/EasyTierDialog";
import { useModalInteractionBoundary } from "../../../src/modal-interaction-boundary";
import { setLanguagePreference } from "../../../src/i18n";
import "../../../src/styles.css";

setLanguagePreference(new URLSearchParams(location.search).get("lang") === "zh" ? "zh" : "en");
function Fixture() {
  useModalInteractionBoundary();
  return <EasyTierDialog onClose={() => { document.body.dataset.closed = "true"; }} />;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
