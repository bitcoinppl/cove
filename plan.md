# Android Implementation Plan

This plan has been split into three files for better organization:

## 📚 Plan Files

### 1. [PLAN_IMPORTANT_CONTEXT_AND_LEARNINGS.md](./PLAN_IMPORTANT_CONTEXT_AND_LEARNINGS.md)
**READ THIS FIRST** before starting any new phase!

Contains:
- Key discoveries about existing Android UI components
- Critical patterns and anti-patterns
- iOS to Android translation guide
- Container architecture patterns
- Common pitfalls to avoid

### 2. [PLAN_TODO.md](./PLAN_TODO.md)
**Check this for remaining work**

Contains:
- All remaining phases (5A through 6)
- Current phase status
- Estimated effort
- What needs to be done

### 3. [PLAN_COMPLETED.md](./PLAN_COMPLETED.md)
**Historical reference only**

Contains:
- Phases 1-7 (all completed)
- Implementation details
- Lessons learned from each phase
- Files created/modified

### 4. [@ANDROID_DEVIATIONS_FROM_IOS.md](./@ANDROID_DEVIATIONS_FROM_IOS.md)
**Platform differences reference**

Contains:
- Consolidated list of all Android deviations from iOS
- Organized by category (UI, Architecture, Features, Technical)
- Rationale for each deviation
- Cross-references to implementation phases

---

## Quick Start

**Starting a new phase?**
1. Read `PLAN_IMPORTANT_CONTEXT_AND_LEARNINGS.md` first
2. Check `PLAN_TODO.md` for what to do
3. Search for existing files BEFORE creating new ones
4. Follow the patterns documented in learnings
5. Update `PLAN_TODO.md` when done

**Remember:** 75% of Android UI already exists. Always search first!

---

## 📝 Documentation Requirements

**When completing ANY phase, ALWAYS document in PLAN_COMPLETED.md:**

### Required Sections:
1. **Implementation Summary** - What was built
2. **Files Created** - List with line counts and descriptions
3. **Files Modified** - List with what changed
4. **Key Features** - Bullet points of main functionality
5. **Lessons Learned** - What we discovered during implementation
6. **Deviations from iOS** - CRITICAL: Document all differences from iOS implementation
7. **Follow-up Items** - What was deferred or needs future work

### Why "Deviations from iOS" is Critical:
- Helps future developers understand intentional platform differences
- Documents when we simplified or enhanced beyond iOS
- Explains technical constraints (e.g., no direct SwiftUI equivalent)
- Tracks features deferred to later phases
- Prevents confusion about why implementations differ

**Example Deviations:**
- "No QR/NFC import in first pass (manual only)"
- "Simpler field layout (no autocomplete suggestions above keyboard)"
- "Focus management handled differently (FocusRequester vs @FocusState)"
- "Used sealed classes instead of enums (more idiomatic Kotlin)"

**✅ Always include this section even if there are no deviations (write "None - follows iOS exactly")**
