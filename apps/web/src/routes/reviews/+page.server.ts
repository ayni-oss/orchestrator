import type { PageServerLoad } from "./$types";
import { getBff } from "$lib/server/bff.js";
import { requestContext, safeError } from "$lib/server/page.js";

export const load: PageServerLoad = async ({ locals, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store" });
  try {
    const bff = getBff();
    const context = requestContext(locals);
    const { reviews, page } = await bff.listReviews({ pageSize: 20, pageToken: url.searchParams.get("page") ?? "" }, context);
    const selected = url.searchParams.get("review");
    const review = selected ? reviews.find((item) => item.id === selected) ?? await bff.getReview(selected,context) : undefined;
    const history = review ? await bff.listAssessments(review.id, { pageSize: 100, pageToken: url.searchParams.get("history") ?? "" }, context) : undefined;
    const evidenceId = url.searchParams.get("assessment");
    const assessment = history?.assessments.find((item) => item.id === evidenceId);
    const evidence = assessment ? (await bff.listEvidence(assessment.id, { pageSize: 32 }, context)).evidence : [];
    return { reviews, page, review, history, evidence, pageToken: url.searchParams.get("page") ?? "", historyToken: url.searchParams.get("history") ?? "" };
  } catch (error) {
    return { reviews: [], evidence: [], error: safeError(error, locals.requestId) };
  }
};
