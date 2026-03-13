import { useCallback } from "react";

import { abortMessage } from "../../../../core/chat/manager";
import type { Session, StoredMessage } from "../../../../core/storage/schemas";
import type { ChatControllerModuleContext } from "./chatControllerShared";

interface UseChatAbortControllerArgs {
  context: ChatControllerModuleContext;
}

function removePlaceholderMessages(messages: StoredMessage[]): StoredMessage[] {
  return messages
    .map((message) => {
      if (!message.id.startsWith("placeholder-")) {
        return message;
      }

      if (message.content.trim().length > 0) {
        return {
          ...message,
          id: crypto.randomUUID(),
        };
      }

      return null;
    })
    .filter((message): message is StoredMessage => message !== null);
}

export function useChatAbortController({ context }: UseChatAbortControllerArgs) {
  const { state, dispatch, messagesRef, persistSession, log } = context;

  const handleAbort = useCallback(async () => {
    if (!state.activeRequestId || !state.session) return;

    try {
      await abortMessage(state.activeRequestId);
      log.info("aborted request", state.activeRequestId);

      const messagesWithoutPlaceholders = removePlaceholderMessages(messagesRef.current);
      const updatedSession: Session = {
        ...state.session,
        messages: messagesWithoutPlaceholders,
        updatedAt: Date.now(),
      };

      try {
        await persistSession(updatedSession);
        messagesRef.current = messagesWithoutPlaceholders;
        dispatch({
          type: "BATCH",
          actions: [
            { type: "SET_SESSION", payload: updatedSession },
            { type: "SET_MESSAGES", payload: messagesWithoutPlaceholders },
          ],
        });
        log.info("successfully saved session after abort");
      } catch (saveError) {
        log.error("failed to save incomplete messages after abort", saveError);
        messagesRef.current = messagesWithoutPlaceholders;
        dispatch({ type: "SET_MESSAGES", payload: messagesWithoutPlaceholders });
      }
    } catch (error) {
      log.error("abort failed", error);

      try {
        const messagesWithoutPlaceholders = removePlaceholderMessages(state.messages);
        const updatedSession: Session = {
          ...state.session,
          messages: messagesWithoutPlaceholders,
          updatedAt: Date.now(),
        };

        await persistSession(updatedSession);
        dispatch({
          type: "BATCH",
          actions: [
            { type: "SET_SESSION", payload: updatedSession },
            { type: "SET_MESSAGES", payload: messagesWithoutPlaceholders },
          ],
        });
      } catch (saveError) {
        log.error("failed to save after abort error", saveError);
        const cleanedMessages = state.messages.filter(
          (message) => !message.id.startsWith("placeholder-") || message.content.trim().length > 0,
        );
        dispatch({ type: "SET_MESSAGES", payload: cleanedMessages });
      }
    }

    dispatch({
      type: "BATCH",
      actions: [
        { type: "SET_SENDING", payload: false },
        { type: "SET_ACTIVE_REQUEST_ID", payload: null },
      ],
    });
  }, [dispatch, log, messagesRef, persistSession, state]);

  return { handleAbort };
}
