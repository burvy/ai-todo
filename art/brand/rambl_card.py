import random, sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter

W, H = 2000, 1000
out = sys.argv[1]
random.seed(7)

img = Image.new("RGB", (W, H), (245, 238, 220))
px = img.load()
for _ in range(220000):  # paper grain
    x, y = random.randrange(W), random.randrange(H)
    r, g, b = px[x, y]
    d = random.randint(-10, 6)
    px[x, y] = (r + d, g + d, b + d)
img = img.filter(ImageFilter.GaussianBlur(0.6))
d = ImageDraw.Draw(img)

for y in range(130, H, 62):  # ruled lines
    d.line([(0, y), (W, y)], fill=(160, 190, 222), width=3)
d.line([(250, 0), (250, H)], fill=(214, 96, 96), width=4)  # margin

title = ImageFont.truetype("C:/Windows/Fonts/segoescb.ttf", 300)
small = ImageFont.truetype("C:/Windows/Fonts/Inkfree.ttf", 64)
ink = (28, 40, 78)
d.text((330, 160), "Rambl", font=title, fill=ink)

lines = ["call mom tomorrow at 3", "lab report Wednesday", "dentist Tuesday 10am"]
for i, text in enumerate(lines):
    y = 626 + i * 124
    d.ellipse([330, y + 2, 370, y + 42], outline=ink, width=4)
    if i == 0:
        d.line([(338, y + 22), (350, y + 34), (366, y + 6)], fill=(40, 90, 170), width=6)
    d.text((400, y - 18), text, font=small, fill=ink)

# pen, drawn on its own layer and rotated
pen = Image.new("RGBA", (900, 120), (0, 0, 0, 0))
p = ImageDraw.Draw(pen)
p.rounded_rectangle([120, 30, 820, 90], radius=30, fill=(32, 52, 110))
p.rectangle([640, 30, 700, 90], fill=(200, 170, 80))
p.rounded_rectangle([700, 22, 860, 98], radius=36, fill=(24, 38, 86))
p.polygon([(120, 30), (120, 90), (20, 60)], fill=(214, 190, 150))
p.polygon([(52, 50), (52, 70), (8, 60)], fill=(30, 30, 30))
p.rectangle([380, 18, 640, 30], fill=(200, 170, 80))
pen = pen.rotate(28, expand=True, resample=Image.BICUBIC)
shadow = Image.new("RGBA", pen.size, (0, 0, 0, 0))
shadow.paste((0, 0, 0, 70), mask=pen.split()[3])
shadow = shadow.filter(ImageFilter.GaussianBlur(12))
img.paste(shadow, (1080 + 24, 300 + 30), shadow)
img.paste(pen, (1080, 300), pen)

img.save(out, optimize=True)
print("saved", out)
