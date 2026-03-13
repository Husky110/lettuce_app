import { useCallback, useEffect, useRef } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { type as getPlatform } from "@tauri-apps/plugin-os";
import { impactFeedback } from "@tauri-apps/plugin-haptics";

import { addChatMessageAttachment } from "../../../../core/chat/manager";
import { generateImage, type ImageGenerationRequest } from "../../../../core/image-generation";
import type { GeneratedImage } from "../../../../core/image-generation";
import { readSettings, SETTINGS_UPDATED_EVENT } from "../../../../core/storage/repo";
import type { ImageAttachment, Session, StoredMessage } from "../../../../core/storage/schemas";
import type { ChatControllerModuleContext } from "./chatControllerShared";

const IMAGE_DIRECTIVE_RE = /<<image:(\{[\s\S]*?\})>>/g;

interface ImageGenConfig {
  modelName: string;
  providerId: string;
  credentialId: string;
}

interface UseChatEnhancementsControllerArgs {
  context: ChatControllerModuleContext;
}

export function useChatEnhancementsController({ context }: UseChatEnhancementsControllerArgs) {
  const { state, dispatch, messagesRef, persistSession } = context;
  const longPressTimerRef = useRef<number | null>(null);
  const processedImageDirectiveMessagesRef = useRef<Set<string>>(new Set());
  const imageGenConfigRef = useRef<ImageGenConfig | null>(null);
  const hapticsEnabledRef = useRef<boolean>(false);
  const hapticIntensityRef = useRef<any>("light");
  const lastHapticTimeRef = useRef<number>(0);
  const platformRef = useRef<string>("");

  useEffect(() => {
    platformRef.current = getPlatform();

    const updateHapticsState = async () => {
      try {
        const settings = await readSettings();
        const acc = settings.advancedSettings?.accessibility;
        hapticsEnabledRef.current = acc?.haptics ?? false;
        hapticIntensityRef.current = acc?.hapticIntensity ?? "light";
      } catch {
        // ignore settings read failures
      }
    };

    void updateHapticsState();
    window.addEventListener(SETTINGS_UPDATED_EVENT, updateHapticsState);
    return () => window.removeEventListener(SETTINGS_UPDATED_EVENT, updateHapticsState);
  }, []);

  useEffect(() => {
    return () => {
      if (longPressTimerRef.current !== null) {
        window.clearTimeout(longPressTimerRef.current);
      }
    };
  }, []);

  const triggerTypingHaptic = useCallback(async () => {
    if (!hapticsEnabledRef.current) return;
    const isMobile = platformRef.current === "android" || platformRef.current === "ios";
    if (!isMobile) return;

    const now = Date.now();
    if (now - lastHapticTimeRef.current < 60) return;

    lastHapticTimeRef.current = now;
    try {
      await impactFeedback(hapticIntensityRef.current);
    } catch {
      // ignore haptics failures
    }
  }, []);

  const resolveDefaultImageGenConfig = useCallback(async (): Promise<ImageGenConfig | null> => {
    if (imageGenConfigRef.current) return imageGenConfigRef.current;

    const settings = await readSettings();
    const imageModels = settings.models.filter((model) => model.outputScopes?.includes("image"));
    const firstModel = imageModels[0];
    if (!firstModel) return null;

    const providerCreds = settings.providerCredentials;
    const provider =
      providerCreds.find(
        (credential) =>
          credential.providerId === firstModel.providerId &&
          credential.label === firstModel.providerLabel,
      ) ??
      providerCreds.find((credential) => credential.providerId === firstModel.providerId) ??
      null;
    if (!provider) return null;

    const config = {
      modelName: firstModel.name,
      providerId: firstModel.providerId,
      credentialId: provider.id,
    };
    imageGenConfigRef.current = config;
    return config;
  }, []);

  const dataUrlFromGeneratedImage = useCallback(
    async (generated: GeneratedImage): Promise<string> => {
      if (generated.url && generated.url.startsWith("data:")) {
        return generated.url;
      }

      const src = generated.url || (generated.filePath ? convertFileSrc(generated.filePath) : null);
      if (!src) {
        throw new Error("Generated image has no url or filePath");
      }

      const response = await fetch(src);
      const blob = await response.blob();

      return await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onloadend = () => resolve(reader.result as string);
        reader.onerror = () => reject(new Error("Failed to read image blob"));
        reader.readAsDataURL(blob);
      });
    },
    [],
  );

  const imageInfoFromDataUrl = useCallback(async (dataUrl: string) => {
    const mimeMatch = dataUrl.match(/^data:([^;]+);base64,/);
    const mimeType = mimeMatch?.[1] ?? "image/png";

    const dimensions = await new Promise<{ width: number; height: number }>((resolve) => {
      const image = new Image();
      image.onload = () => resolve({ width: image.width, height: image.height });
      image.onerror = () => resolve({ width: 0, height: 0 });
      image.src = dataUrl;
    });

    return {
      mimeType,
      width: dimensions.width || undefined,
      height: dimensions.height || undefined,
    };
  }, []);

  const parseImageDirectives = useCallback((content: string) => {
    const directives: Array<{
      prompt: string;
      size?: string;
      n?: number;
      quality?: string;
      style?: string;
    }> = [];

    IMAGE_DIRECTIVE_RE.lastIndex = 0;
    const cleanContent = content.replace(IMAGE_DIRECTIVE_RE, (fullMatch, jsonStr) => {
      try {
        const parsed = JSON.parse(jsonStr);
        const prompt = typeof parsed?.prompt === "string" ? parsed.prompt.trim() : "";
        if (!prompt) return "";

        directives.push({
          prompt,
          size: typeof parsed?.size === "string" ? parsed.size : undefined,
          n: typeof parsed?.n === "number" ? parsed.n : undefined,
          quality: typeof parsed?.quality === "string" ? parsed.quality : undefined,
          style: typeof parsed?.style === "string" ? parsed.style : undefined,
        });
      } catch {
        return fullMatch;
      }

      return "";
    });

    return { cleanContent: cleanContent.trim(), directives };
  }, []);

  const parseSize = useCallback((size?: string) => {
    if (!size) return null;
    const match = size.match(/^(\d+)\s*x\s*(\d+)$/i);
    if (!match) return null;

    const width = Number(match[1]);
    const height = Number(match[2]);
    if (!Number.isFinite(width) || !Number.isFinite(height) || width <= 0 || height <= 0) {
      return null;
    }

    return { width, height };
  }, []);

  const runInChatImageGeneration = useCallback(
    async (assistantMessageId: string) => {
      if (!state.session || !state.character) return;
      if (processedImageDirectiveMessagesRef.current.has(assistantMessageId)) return;

      const currentMessage = messagesRef.current.find(
        (message) => message.id === assistantMessageId,
      );
      if (!currentMessage) return;

      const { cleanContent, directives } = parseImageDirectives(currentMessage.content);
      if (directives.length === 0) return;

      const config = await resolveDefaultImageGenConfig();
      if (!config) return;

      processedImageDirectiveMessagesRef.current.add(assistantMessageId);

      const placeholderAttachments: ImageAttachment[] = [];
      for (const directive of directives) {
        const count = Math.max(1, Math.min(4, directive.n ?? 1));
        const dimensions = parseSize(directive.size) ?? parseSize("1024x1024");
        for (let index = 0; index < count; index++) {
          placeholderAttachments.push({
            id: crypto.randomUUID(),
            data: "",
            mimeType: "image/webp",
            width: dimensions?.width,
            height: dimensions?.height,
          });
        }
      }

      const updatedMessage: StoredMessage = {
        ...currentMessage,
        content: cleanContent,
        attachments: [...(currentMessage.attachments ?? []), ...placeholderAttachments],
      };

      const updatedMessages = messagesRef.current.map((message) =>
        message.id === assistantMessageId ? updatedMessage : message,
      );
      messagesRef.current = updatedMessages;

      const updatedSession: Session = {
        ...state.session,
        messages: updatedMessages,
        updatedAt: Date.now(),
      };

      dispatch({
        type: "BATCH",
        actions: [
          { type: "SET_MESSAGES", payload: updatedMessages },
          { type: "SET_SESSION", payload: updatedSession },
        ],
      });

      try {
        await persistSession(updatedSession);
      } catch {
        // allow generation to continue even if placeholder persistence fails
      }

      for (
        let directiveIndex = 0, placeholderIndex = 0;
        directiveIndex < directives.length;
        directiveIndex++
      ) {
        const directive = directives[directiveIndex];
        const count = Math.max(1, Math.min(4, directive.n ?? 1));
        const placeholdersForDirective = placeholderAttachments.slice(
          placeholderIndex,
          placeholderIndex + count,
        );
        placeholderIndex += count;

        const request: ImageGenerationRequest = {
          prompt: directive.prompt,
          model: config.modelName,
          providerId: config.providerId,
          credentialId: config.credentialId,
          size: directive.size ?? "1024x1024",
          n: count,
          quality: directive.quality,
          style: directive.style,
        };

        try {
          const response = await generateImage(request);
          const images = response.images.slice(0, placeholdersForDirective.length);

          for (let index = 0; index < images.length; index++) {
            const placeholderId = placeholdersForDirective[index]?.id;
            if (!placeholderId) continue;

            const dataUrl = await dataUrlFromGeneratedImage(images[index]);
            const info = await imageInfoFromDataUrl(dataUrl);

            const updated = await addChatMessageAttachment({
              sessionId: state.session.id,
              characterId: state.character.id,
              messageId: assistantMessageId,
              role: "assistant",
              attachmentId: placeholderId,
              base64Data: dataUrl,
              mimeType: info.mimeType,
              width: info.width,
              height: info.height,
            });

            const nextMessages = messagesRef.current.map((message) =>
              message.id === updated.id ? updated : message,
            );
            messagesRef.current = nextMessages;
            dispatch({ type: "SET_MESSAGES", payload: nextMessages });
          }
        } catch (error) {
          console.error("In-chat image generation failed:", error);
          const placeholderIds = new Set(
            placeholdersForDirective.map((placeholder) => placeholder.id),
          );
          const latestMessage = messagesRef.current.find(
            (message) => message.id === assistantMessageId,
          );
          if (!latestMessage || placeholderIds.size === 0) continue;

          const cleanedMessage: StoredMessage = {
            ...latestMessage,
            attachments: (latestMessage.attachments ?? []).filter(
              (attachment) => !placeholderIds.has(attachment.id),
            ),
          };
          const nextMessages = messagesRef.current.map((message) =>
            message.id === cleanedMessage.id ? cleanedMessage : message,
          );
          messagesRef.current = nextMessages;

          const nextSession: Session = {
            ...state.session,
            messages: nextMessages,
            updatedAt: Date.now(),
          };

          dispatch({
            type: "BATCH",
            actions: [
              { type: "SET_MESSAGES", payload: nextMessages },
              { type: "SET_SESSION", payload: nextSession },
            ],
          });

          try {
            await persistSession(nextSession);
          } catch {
            // leave cleaned UI state in memory if persistence fails
          }
        }
      }
    },
    [
      dataUrlFromGeneratedImage,
      dispatch,
      imageInfoFromDataUrl,
      messagesRef,
      parseImageDirectives,
      parseSize,
      persistSession,
      resolveDefaultImageGenConfig,
      state.character,
      state.session,
    ],
  );

  const initializeLongPressTimer = useCallback((timer: number | null) => {
    if (timer === null) {
      if (longPressTimerRef.current !== null) {
        window.clearTimeout(longPressTimerRef.current);
        longPressTimerRef.current = null;
      }
      return;
    }

    longPressTimerRef.current = timer;
  }, []);

  return {
    initializeLongPressTimer,
    runInChatImageGeneration,
    triggerTypingHaptic,
  };
}
