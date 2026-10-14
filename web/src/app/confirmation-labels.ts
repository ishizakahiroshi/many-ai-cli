import { t, type I18nVars } from '../i18n.js';

export type ConfirmationAction = 'update' | 'delete' | 'save' | 'stop' | 'stopAll' | 'stopHub' | 'reset' | 'revoke' | 'disconnect' | 'close' | 'continue' | 'restore' | 'launch' | 'push' | 'pull' | 'disable' | 'clear' | 'remove' | 'approve' | 'retrieve' | 'add' | 'send' | 'move' | 'discard' | 'overwrite' | 'import' | 'export' | 'install' | 'edit' | 'copy';
export interface ConfirmationLabels { question: string; decline: string; accept: string }

const INFLECTED_ACTIONS = new Set<ConfirmationAction>(['close', 'continue']);

// Explicit action metadata, never a guess extracted from a question or body.
// Late-bound t() keeps locale changes effective and avoids module-init DOM reads.
export function confirmationLabels(action: ConfirmationAction, vars?: I18nVars): ConfirmationLabels {
  if (INFLECTED_ACTIONS.has(action)) {
    return {
      question: t(`ui_confirm_${action}_question`, vars),
      decline: t(`ui_confirm_${action}_decline`, vars),
      accept: t(`ui_confirm_${action}_accept`, vars),
    };
  }
  const values = { ...vars, action: t(`ui_action_${action}`, vars) };
  return {
    question: t('ui_confirm_question', values),
    decline: t('ui_confirm_decline', values),
    accept: t('ui_confirm_accept', values),
  };
}
