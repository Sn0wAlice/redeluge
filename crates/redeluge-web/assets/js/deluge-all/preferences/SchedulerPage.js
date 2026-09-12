/**
 * Deluge.preferences.Scheduler.js
 *
 * Copyright (c) 2026 the redeluge contributors
 *
 * This file is part of redeluge and is licensed under GNU General Public License 3.0, or later,
 * with the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Scheduler
 * @extends Ext.Panel
 *
 * The weekly grid, and the limits that apply in its slow hours.
 *
 * This was the Scheduler plugin. It is a feature of the daemon now, held in
 * the `scheduler` key of core.conf, so the page reads and writes that one
 * dictionary rather than binding flat keys through the options manager.
 *
 * The grid is 24 rows of 7 and is indexed [hour][weekday] with Monday as 0,
 * which is the shape the plugin stored. Getting the two indices the wrong way
 * round would silently apply Tuesday's rules on Wednesday.
 */
Deluge.preferences.Scheduler = Ext.extend(Ext.Panel, {
    border: false,
    title: _('Schedule'),
    header: false,
    layout: 'form',
    autoScroll: true,

    // 0 full speed, 1 slow, 2 stopped. The order the daemon reads.
    STATES: 3,

    initComponent: function () {
        Deluge.preferences.Scheduler.superclass.initComponent.call(this);

        this.grid = [];
        for (var hour = 0; hour < 24; hour++) {
            this.grid[hour] = [0, 0, 0, 0, 0, 0, 0];
        }

        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Schedule'),
            autoHeight: true,
            labelWidth: 1,
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.enabled = fieldset.add({
            xtype: 'checkbox',
            fieldLabel: '',
            labelSeparator: '',
            boxLabel: _('Enable the schedule'),
            name: 'scheduler_enabled',
        });

        this.gridPanel = fieldset.add({
            xtype: 'panel',
            border: false,
            html: this.gridHtml(),
            style: 'padding-top: 8px;',
        });
        this.gridPanel.on('afterrender', this.onGridRender, this);

        var limits = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Slow Settings'),
            autoHeight: true,
            labelWidth: 200,
            defaultType: 'spinnerfield',
            style: 'padding-top: 5px; margin-bottom: 0px;',
        });

        this.low_down = limits.add({
            fieldLabel: _('Maximum Download Speed (KiB/s):'),
            labelSeparator: '',
            name: 'low_down',
            width: 80,
            value: -1,
            decimalPrecision: 1,
            minValue: -1,
        });
        this.low_up = limits.add({
            fieldLabel: _('Maximum Upload Speed (KiB/s):'),
            labelSeparator: '',
            name: 'low_up',
            width: 80,
            value: -1,
            decimalPrecision: 1,
            minValue: -1,
        });
        this.low_active = limits.add({
            fieldLabel: _('Active Torrents:'),
            labelSeparator: '',
            name: 'low_active',
            width: 80,
            value: -1,
            decimalPrecision: 0,
            minValue: -1,
        });
        this.low_active_down = limits.add({
            fieldLabel: _('Active Downloading:'),
            labelSeparator: '',
            name: 'low_active_down',
            width: 80,
            value: -1,
            decimalPrecision: 0,
            minValue: -1,
        });
        this.low_active_up = limits.add({
            fieldLabel: _('Active Seeding:'),
            labelSeparator: '',
            name: 'low_active_up',
            width: 80,
            value: -1,
            decimalPrecision: 0,
            minValue: -1,
        });

        this.on('show', this.onPageShow, this);
    },

    /**
     * The grid, as a table. A cell carries its hour and day so the click
     * handler does not have to work them out from the DOM.
     */
    gridHtml: function () {
        var days = [
            _('Mon'),
            _('Tue'),
            _('Wed'),
            _('Thu'),
            _('Fri'),
            _('Sat'),
            _('Sun'),
        ];

        var html = '<table class="x-deluge-schedule"><thead><tr><th></th>';
        for (var day = 0; day < 7; day++) {
            html += '<th>' + days[day] + '</th>';
        }
        html += '</tr></thead><tbody>';

        for (var hour = 0; hour < 24; hour++) {
            html += '<tr><th>' + (hour < 10 ? '0' : '') + hour + ':00</th>';
            for (day = 0; day < 7; day++) {
                html +=
                    '<td class="x-deluge-schedule-0" data-hour="' +
                    hour +
                    '" data-day="' +
                    day +
                    '">&nbsp;</td>';
            }
            html += '</tr>';
        }
        html += '</tbody></table>';
        html +=
            '<div class="x-deluge-schedule-key">' +
            '<span class="x-deluge-schedule-0">&nbsp;&nbsp;&nbsp;</span> ' +
            _('Full speed') +
            ' &nbsp; <span class="x-deluge-schedule-1">&nbsp;&nbsp;&nbsp;</span> ' +
            _('Slow') +
            ' &nbsp; <span class="x-deluge-schedule-2">&nbsp;&nbsp;&nbsp;</span> ' +
            _('Stopped') +
            '</div>';
        return html;
    },

    onGridRender: function (panel) {
        panel.body.on('click', this.onCellClick, this, { delegate: 'td' });
    },

    /**
     * A click cycles the cell through the three states.
     */
    onCellClick: function (event, target) {
        var cell = Ext.get(target);
        var hour = parseInt(cell.getAttribute('data-hour'), 10);
        var day = parseInt(cell.getAttribute('data-day'), 10);
        if (isNaN(hour) || isNaN(day)) return;

        var next = (this.grid[hour][day] + 1) % this.STATES;
        this.grid[hour][day] = next;
        this.paintCell(cell, next);
    },

    paintCell: function (cell, state) {
        for (var value = 0; value < this.STATES; value++) {
            cell.removeClass('x-deluge-schedule-' + value);
        }
        cell.addClass('x-deluge-schedule-' + state);
    },

    /**
     * Paints the whole grid from this.grid.
     */
    repaint: function () {
        if (!this.gridPanel.body) return;
        this.gridPanel.body.select('td').each(
            function (cell) {
                var hour = parseInt(cell.getAttribute('data-hour'), 10);
                var day = parseInt(cell.getAttribute('data-day'), 10);
                if (isNaN(hour) || isNaN(day)) return;
                this.paintCell(cell, this.grid[hour][day]);
            }.bind(this)
        );
    },

    // The card layout fires `show` whenever the window opens on this page and
    // again on every switch back to it. Fetching each time would be a call per
    // click; once is enough, and the Apply button writes rather than reads.
    onPageShow: function () {
        if (this.loaded) return;
        this.loaded = true;
        deluge.client.core.get_config({
            success: this.onGotConfig,
            scope: this,
        });
    },

    onGotConfig: function (config) {
        var settings = config['scheduler'] || {};
        this.enabled.setValue(settings['enabled'] === true);
        this.low_down.setValue(Ext.value(settings['low_down'], -1));
        this.low_up.setValue(Ext.value(settings['low_up'], -1));
        this.low_active.setValue(Ext.value(settings['low_active'], -1));
        this.low_active_down.setValue(Ext.value(settings['low_active_down'], -1));
        this.low_active_up.setValue(Ext.value(settings['low_active_up'], -1));

        var stored = settings['button_state'];
        for (var hour = 0; hour < 24; hour++) {
            for (var day = 0; day < 7; day++) {
                var value = 0;
                if (stored && stored[hour] && typeof stored[hour][day] == 'number') {
                    value = stored[hour][day];
                }
                this.grid[hour][day] = value % this.STATES;
            }
        }
        this.repaint();
    },

    /**
     * The whole dictionary goes back at once: core.set_config replaces the key
     * rather than merging into it.
     */
    onApply: function () {
        // Preferences applies every page, including ones nobody opened.
        // Until this page has read the stored settings its fields hold
        // defaults and its grid is empty, and writing that back would replace
        // what is configured with nothing.
        if (!this.loaded) return;

        deluge.client.core.set_config({
            scheduler: {
                enabled: this.enabled.getValue(),
                button_state: this.grid,
                low_down: Deluge.number(this.low_down.getValue(), -1),
                low_up: Deluge.number(this.low_up.getValue(), -1),
                low_active: Deluge.number(this.low_active.getValue(), -1),
                low_active_down: Deluge.number(this.low_active_down.getValue(), -1),
                low_active_up: Deluge.number(this.low_active_up.getValue(), -1),
            },
        });
    },

    onOk: function () {
        this.onApply();
    },
});
