import re
import unittest
import xml.etree.ElementTree as ET

import build

FOLDER = build.ROOT / "docs" / "images" / "diagrams"
SVG = "{http://www.w3.org/2000/svg}"
# Each diagram and the pages that show it.
PLACES = {
    "task-flow": ["site/home.md", "docs/concepts.md"],
    "pieces": ["docs/daemon/README.md"],
    "workflow": ["GETTING_STARTED.md"],
    "ask": ["docs/daemon/waits.md"],
    "runners": ["docs/daemon/runners.md"],
}


def alt_texts(page, name):
    text = (build.ROOT / page).read_text(encoding="utf-8")
    file = re.escape(f"images/diagrams/{name}.svg")
    markdown = re.findall(rf"!\[([^\]]*)\]\([^)\s]*{file}\)", text)
    html = [m[1] or "" for m in re.finditer(rf'<img\b(?=[^>]*\bsrc="[^"]*{file}")(?:[^>]*\balt="([^"]*)")?[^>]*>', text)]
    return markdown + html


class Diagrams(unittest.TestCase):
    def test_every_diagram_file_is_placed_in_a_page(self):
        self.assertEqual(sorted(p.stem for p in FOLDER.glob("*.svg")), sorted(PLACES))

    def test_each_diagram_names_and_describes_itself_and_draws_a_dark_palette(self):
        for name in PLACES:
            with self.subTest(name):
                text = (FOLDER / f"{name}.svg").read_text(encoding="utf-8")
                root = ET.fromstring(text)
                self.assertEqual([child.tag for child in root][:2], [f"{SVG}title", f"{SVG}desc"])
                self.assertGreater(len(root[0].text.strip()), 10)
                self.assertGreater(len(root[1].text.strip()), 80)
                self.assertIn("@media (prefers-color-scheme: dark)", text)

    def test_each_diagram_draws_the_graphite_colors_of_the_token_files(self):
        for name in PLACES:
            with self.subTest(name):
                text = (FOLDER / f"{name}.svg").read_text(encoding="utf-8")
                self.assertEqual(len(build.PALETTE.findall(text)), 2)
                self.assertEqual(text, build.diagram_palette(text), "run python3 site/build.py diagrams")

    def test_each_diagram_keeps_its_text_source_beside_it(self):
        for name in PLACES:
            with self.subTest(name):
                self.assertGreater(len((FOLDER / f"{name}.mmd").read_text(encoding="utf-8").strip()), 80)

    def test_each_page_shows_its_diagram_with_alt_text(self):
        for name, pages in PLACES.items():
            for page in pages:
                with self.subTest(f"{name} in {page}"):
                    alts = alt_texts(page, name)
                    self.assertEqual(len(alts), 1)
                    self.assertGreater(len(alts[0]), 60)


if __name__ == "__main__":
    unittest.main()
