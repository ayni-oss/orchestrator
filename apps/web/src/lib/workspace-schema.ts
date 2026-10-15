import { projectSchema } from "./project-schema.js";
export const workspaceSchema=projectSchema.omit({workspaceId:true,goal:true});
