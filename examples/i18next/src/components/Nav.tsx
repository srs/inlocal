import { useTranslation } from "react-i18next";

export function Nav({ name }: { name: string }) {
  const { t } = useTranslation("common");

  return (
    <nav>
      <a href="/parts">{t("nav.parts")}</a>
      <a href="/printers">{t("nav.printers")}</a>
      <span>{t("greeting", { name })}</span>
    </nav>
  );
}
