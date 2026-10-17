# contour-1 not hosted

**Decided:** 2026-10-17

**Choice:** `contour-1`, the contour style of C. Chan, F. Durand and P. Isola, "Learning to generate line drawings that convey geometry and semantics", CVPR 2022, is not hosted in `plotline-models` and gets no catalogue entry. The model stage gains no whole-image mode, since no listed model needs one. This amends [model catalogue](2026-10-16-model-catalogue.md), which planned `contour-1` as the second entry.

**Reason:** the spec said the model's drawings come from the Contour Drawings dataset under CC BY 3.0. They do not. The dataset's project page, http://www.cs.cmu.edu/~mengtial/proj/sketch/ (M. Li, Z. Lin, R. Měch, E. Yumer and D. Ramanan, "Photo-Sketching: inferring contour drawings from images", WACV 2019), says it "is licensed under CC BY-NC-SA (Attribution-NonCommercial-ShareAlike)" and links CC BY-NC-SA 4.0. The Internet Archive's copies of January 2019 and March 2022, before the weights in the authors' `model.zip` were saved in July 2022, say the same. The paper's other training set is a subset of 10,000 COCO photos, which keep their Flickr licences.

`teed-1` is listed with non-commercial training data, so the non-commercial term alone would not rule `contour-1` out. Share-alike might: if the trained weights count as adapted material of the drawings, sharing them requires the same CC BY-NC-SA licence. That clashes with hosting them under the MIT licence their repository gives them. Whether trained weights are adapted material is unsettled, and plotline takes the reading with the least exposure.

The conversion itself worked, so a later model with clean terms that also normalises over the whole image would not need it measured again. The file was 17.2 MB. It matched the authors' network to within 0.07/255. Its tiles differed from a whole-image run by up to 254.9/255 at every overlap from 32 to 192, so such a model needs a whole-image mode. The converted file was deleted and nothing was pushed.
