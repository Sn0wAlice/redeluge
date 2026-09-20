/**
 * Deluge.DurationField.js
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 *
 * A delay, entered as days and hours, stored as hours.
 *
 * Every rule that waits before doing something — move it, remove it, give up
 * on it — is stored as a number of hours, because that is what the daemon
 * counts in. A single hours box is fine for "four hours" and useless for the
 * delays that matter on a box that has been up for a year: a tracker that
 * wants a month of seeding is 720, and nobody types 720 confidently or reads
 * it back as a month.
 *
 * So the box is two boxes, and the value is still one number. `getValue` and
 * `setValue` speak hours exactly as the spinner they replace did, which is why
 * swapping one in changes only the field's xtype at the call site.
 *
 * Anything typed in the hours box that is worth a day or more is carried into
 * the days box rather than marked invalid: 36 is a real way to say a day and a
 * half, and correcting it in front of somebody is friendlier than refusing it.
 */
Ext.ns('Deluge');

/**
 * @class Deluge.DurationField
 * @extends Ext.form.CompositeField
 * @xtype durationfield
 */
Deluge.DurationField = Ext.extend(Ext.form.CompositeField, {
    /**
     * @cfg {Number} maxHours The longest delay this field will hold. Ten
     * years, which is what the daemon clamps its own delays to.
     */
    maxHours: 87600,

    /**
     * @cfg {Number} decimalPrecision Places kept in the hours box. Half an
     * hour is a delay somebody asks for; half a minute is not.
     */
    decimalPrecision: 1,

    // One error under two boxes reads as one field, which is what this is.
    combineErrors: false,
    labelSeparator: '',

    initComponent: function () {
        this.days = new Ext.ux.form.SpinnerField({
            width: 55,
            decimalPrecision: 0,
            minValue: 0,
            maxValue: Math.floor(this.maxHours / 24),
            incrementValue: 1,
            value: 0,
        });
        this.hours = new Ext.ux.form.SpinnerField({
            width: 55,
            decimalPrecision: this.decimalPrecision,
            minValue: 0,
            // Not 23: the carry does the correcting, and a field that goes red
            // while you are still typing the second digit is a field that
            // fights you.
            maxValue: this.maxHours,
            incrementValue: 1,
            value: 0,
        });

        this.items = [
            this.days,
            this.suffix(_('days')),
            this.hours,
            this.suffix(_('hours')),
        ];

        Deluge.DurationField.superclass.initComponent.call(this);

        // A spinner fires `spin` as it is clicked and `change` when it is left
        // having been typed in; both are the value changing, and whoever holds
        // this field wants to hear about it once, from the field.
        Ext.each(
            [this.days, this.hours],
            function (box) {
                box.on('spin', this.onPartChanged, this);
                box.on('change', this.onPartChanged, this);
            },
            this
        );
    },

    /**
     * The word after a box, which is the only thing saying which box is which.
     */
    suffix: function (text) {
        return {
            xtype: 'label',
            text: text,
            style: 'padding: 3px 4px 0 0; opacity: 0.72;',
        };
    },

    onPartChanged: function () {
        var value = this.getValue();
        // Read before the re-split, which is what moves `lastValue` on.
        var previous = this.lastValue;
        // Re-split rather than leave 36 sitting in the hours box: the value is
        // the same either way, and the form should show what it means.
        this.setValue(value);
        if (value !== previous) {
            this.fireEvent('change', this, value, previous);
        }
    },

    /**
     * The delay, in hours, which is how it is stored.
     */
    getValue: function () {
        var total =
            Deluge.number(this.days.getValue(), 0) * 24 +
            Deluge.number(this.hours.getValue(), 0);
        // Two boxes of tenths add up to float noise otherwise: 0.1 + 0.2 hours
        // would be written to the configuration as 0.30000000000000004.
        total = Math.round(total * 100) / 100;
        return Math.min(Math.max(total, 0), this.maxHours);
    },

    /**
     * @param {Number} value The delay in hours, as the daemon stores it
     */
    setValue: function (value) {
        var total = Math.min(
            Math.max(Deluge.number(value, 0), 0),
            this.maxHours
        );
        var days = Math.floor(total / 24);
        this.days.setValue(days);
        this.hours.setValue(Math.round((total - days * 24) * 100) / 100);
        this.value = total;
        this.lastValue = total;
        return this;
    },
});

Ext.reg('durationfield', Deluge.DurationField);
