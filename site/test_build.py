import tempfile
import unittest
from pathlib import Path

import build

PAGES = {p for _, ps in build.chapters() for p in ps}


class RewriteTarget(unittest.TestCase):
    def check(self, target, source, expected):
        self.assertEqual(build.rewrite_target(target, source, PAGES), expected)

    def test_keeps_a_link_between_pages_of_the_book(self):
        self.check("../reference/events.md#envelope", "docs/intake/envelope.md", "../reference/events.md#envelope")

    def test_points_a_link_to_a_folder_at_its_readme(self):
        self.check("docs/reference/", "README.md", "docs/reference/README.md")

    def test_points_the_root_readme_at_the_home_page(self):
        self.check("../README.md", "docs/prd.md", "../index.md")

    def test_keeps_a_copied_file_under_docs(self):
        self.check("docs/images/web-view-ask-390-light.png", "README.md", "docs/images/web-view-ask-390-light.png")

    def test_points_a_file_outside_the_book_at_github(self):
        self.check("DESIGN.md#copy", "README.md", f"{build.REPO}/blob/main/DESIGN.md#copy")
        self.check("../../examples/", "docs/reference/README.md", f"{build.REPO}/tree/main/examples")

    def test_leaves_urls_and_anchors_alone(self):
        self.check("https://lucide.dev", "DESIGN.md", "https://lucide.dev")
        self.check("#log", "docs/reference/factory.md", "#log")

    def test_keeps_a_query_apart_from_the_path(self):
        self.check("GETTING_STARTED.md?mode=print#concepts", "README.md", "GETTING_STARTED.md?mode=print#concepts")

    def test_skips_code(self):
        text = "\n".join([
            "[a](DESIGN.md) and `[b](DESIGN.md)` and [`e`](DESIGN.md)",
            "````md",
            "```",
            "[c](DESIGN.md)",
            "```",
            "````",
            "    [d](DESIGN.md)",
        ])
        out = build.rewrite_links(text, "README.md", PAGES).split("\n")
        design = f"{build.REPO}/blob/main/DESIGN.md"
        self.assertEqual(out[0], f"[a]({design}) and `[b](DESIGN.md)` and [`e`]({design})")
        self.assertEqual(out[3], "[c](DESIGN.md)")
        self.assertEqual(out[6], "    [d](DESIGN.md)")


class Staging(unittest.TestCase):
    def test_refuses_a_symlinked_build_folder(self):
        build_dir = build.SITE / "build"
        if build_dir.exists() or build_dir.is_symlink():
            self.skipTest("site/build exists; this test needs a clean checkout")
        target = Path(tempfile.mkdtemp())
        build_dir.symlink_to(target)
        try:
            with self.assertRaises(SystemExit):
                build.main()
            self.assertEqual(list(target.iterdir()), [])
        finally:
            build_dir.unlink()
            target.rmdir()


class Chapters(unittest.TestCase):
    def test_every_doc_page_is_listed_once(self):
        listed = [p for _, ps in build.chapters() for p in ps]
        self.assertEqual(len(listed), len(set(listed)))
        docs = build.ROOT / "docs"
        on_disk = {p.relative_to(build.ROOT).as_posix() for p in docs.glob("*/*.md")} | {
            p.relative_to(build.ROOT).as_posix() for p in docs.glob("*.md")
        }
        self.assertEqual(on_disk - set(listed), set())

    def test_the_adapters_group_lists_herdr_then_github(self):
        adapters = dict(build.chapters())["Adapters"]
        self.assertEqual(adapters, ["docs/adapters/herdr.md", "docs/adapters/github.md"])

    def test_the_split_pages_sit_in_their_diataxis_groups(self):
        groups = dict(build.chapters())
        self.assertIn("docs/installing-the-herdr-adapter.md", groups["How-to guides"])
        self.assertIn("docs/reference/herdr-panels.md", groups["Reference"])
        self.assertIn("docs/reference/tools.md", groups["Reference"])
        self.assertIn("docs/daemon/tools.md", groups["How it works"])

    def test_home_includes_every_section_it_names(self):
        home = build.home(PAGES)
        self.assertNotIn("<!-- include", home)
        self.assertIn("docs/concepts.md", home)


if __name__ == "__main__":
    unittest.main()


class Intro(unittest.TestCase):
    def test_the_home_intro_leaves_out_the_readme_tagline(self):
        intro = build.section("README.md", "intro")
        self.assertNotIn("<!-- tagline", intro)
        self.assertNotIn("Build your own agent factory", intro)
        self.assertTrue(intro.startswith("Priory runs coding agents"), intro[:80])
