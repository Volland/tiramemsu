# Article: Validators that ask "what if"

A hands-on tour (about 1,700 words, no images) of ELPI in Tiramemsu, for users rather than implementers: a question, a hypothetical answer with `=>`, policies passed as predicates, `pi` for any member, a validator enforced at commit, and materialized conclusions. It is marked **Preview** until ELPI ships in a release. It appears on the website as `site/articles/elpi-validators.html`. The engineering companion is `article/elpi/`.

| File | What it is |
|---|---|
| `elpi-validators.md` | The article, the source of truth |
| `build-site.py` | Rebuilds the website page (needs `pandoc`): `python3 build-site.py` |

## Where the claims come from

Every program in the article is a test in `crates/tiramemsu/tests/elpi.rs`:
- sections 2 to 4: `article_hypothetical_reach`, `article_policies_as_predicates`, `article_pi_any_member`;
- section 5: `second_ceo_refused`, `unrelated_write_commits`, `retraction_seeds_a_check`, and the property test in `elpi_props.rs`;
- section 6: `grandparents_materialized`, `premise_retracted`, `library_edited`, `no_self_support`.

A change to the prelude or the builtins means re-running those tests and updating the text.
