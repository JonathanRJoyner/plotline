# Decision and changelog files

**Decided:** 2026-10-16

**Choice:** New decisions are files in `docs/decisions/` and new changelog entries are files in `changelog.d/`, instead of rows appended to the brief's decision table and lines added under Unreleased in `CHANGELOG.md`. At a release, the changelog files move into `CHANGELOG.md` under the version heading and are deleted. The brief's table keeps every decision made before this date.

**Reason:** On 2026-10-16 up to six pull requests were open at once, and almost every merge left the rest conflicting in `CHANGELOG.md` and `docs/BRIEF.md`, because each one added to the same place: the end of the decision table and the top of the changelog. Each conflict cost a merge of main and a full CI run of about 20 minutes, even though the resolution was always to keep both sides. One file per entry can't conflict. GitHub ignores `.gitattributes` merge drivers such as `union` when it decides whether a pull request can merge, so that cheaper fix wasn't available. Decision files follow M. Nygard's architecture decision records (2011).
