import { Trans, useTranslation } from "react-i18next";

export function Printer({ state, file }: { state: string; file: string }) {
  const { t } = useTranslation("printers");

  return (
    <div>
      <span>{t(`status.${state}`, { file })}</span>
      <button>{t("actions.pause")}</button>
      <Trans i18nKey="common:items" values={{ count: 2 }} />
      <small>{t("actions.cancel")}</small>
    </div>
  );
}
