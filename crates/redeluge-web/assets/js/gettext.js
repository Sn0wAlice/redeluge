/*
 * gettext.js
 *
 * redeluge is English only, so translation is the identity function.
 *
 * The Python Web UI shipped a generated catalogue here in which every entry
 * mapped a string to a Mako expression, and the server rendered the file
 * through Mako on each request to substitute real translations. That made a
 * static asset into a server-rendered one, and serving it unrendered turned
 * every label in the interface into the raw Mako expression instead of text.
 *
 * Dropping translation removes the render step, the catalogue, the extraction
 * tool and a whole class of bug. The API is kept exactly as the front end
 * expects it, so no JavaScript needed changing: GetText.add is accepted and
 * ignored, and _() returns what it was given.
 *
 * A test asserts that no shipped asset still carries an unrendered marker, so
 * this comment deliberately describes the old catalogue rather than quoting it.
 */
GetText = {
    maps: {},
    add: function (string, translation) {
        // Accepted so third-party code that populates a catalogue still runs.
        this.maps[string] = translation;
    },
    get: function (string) {
        return string;
    },
};

function _(string) {
    return GetText.get(string);
}

/*
 * Plural form. English has two, and the front end passes both, so choosing
 * between them needs no catalogue.
 */
function ngettext(singular, plural, count) {
    return count === 1 ? singular : plural;
}

var _n = ngettext;
