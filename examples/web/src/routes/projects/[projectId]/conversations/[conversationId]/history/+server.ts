import type {RequestHandler} from "./$types";
import {conversationHistory} from "$lib/server/conversation-history.js";
export const GET:RequestHandler=event=>conversationHistory(event);
